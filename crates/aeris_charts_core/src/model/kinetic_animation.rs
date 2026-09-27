//! Independently implemented kinetic (momentum) scroll behavior, informed by observable public
//! interaction semantics. The release speed is the distance-weighted average of up to three consecutive
//! same-direction segments, and the coast follows `start + speed * (c^t - 1) / ln(c)`.
//!
//! The model is platform-free: the host feeds pointer samples (`add_position`) during a drag,
//! engages the coast on release (`start`), and drives it from its own frame scheduler
//! (`position`/`finished`). All times are milliseconds in the host's clock.

/// The last sample may lag the release by at most this.
const MAX_START_DELAY_MS: f64 = 50.0;

#[derive(Clone, Copy, Debug)]
struct TimeAndPosition {
    time: f64,
    position: f64,
}

/// reference `speedPxPerMSec`: segment speed, magnitude clamped to `max_speed`.
fn speed_px_per_msec(a: TimeAndPosition, b: TimeAndPosition, max_speed: f64) -> f64 {
    let speed = (a.position - b.position) / (a.time - b.time);
    speed.signum() * speed.abs().min(max_speed)
}

/// reference `durationMSec`: time until the remaining travel shrinks to `epsilon` px.
fn duration_msec(speed: f64, dumping_coeff: f64, epsilon: f64) -> f64 {
    let ln_dumping_coeff = dumping_coeff.ln();
    ((epsilon * ln_dumping_coeff) / -speed).ln() / ln_dumping_coeff
}

/// reference `KineticAnimation`. The tuning knobs are host-supplied (reference
/// `KineticScrollOptions`): min/max speed in px/ms, the per-ms velocity damping factor, and the
/// minimum travel between samples in px.
#[derive(Clone, Debug)]
pub struct KineticAnimation {
    position1: Option<TimeAndPosition>,
    position2: Option<TimeAndPosition>,
    position3: Option<TimeAndPosition>,
    position4: Option<TimeAndPosition>,
    animation_start_position: Option<TimeAndPosition>,
    duration_msecs: f64,
    speed_px_per_msec: f64,
    min_move: f64,
    min_speed: f64,
    max_speed: f64,
    dumping_coeff: f64,
    epsilon_distance: f64,
}

impl KineticAnimation {
    pub fn new(min_speed: f64, max_speed: f64, dumping_coeff: f64, min_move: f64) -> Self {
        Self {
            position1: None,
            position2: None,
            position3: None,
            position4: None,
            animation_start_position: None,
            duration_msecs: 0.0,
            speed_px_per_msec: 0.0,
            min_move,
            min_speed,
            max_speed,
            dumping_coeff,
            epsilon_distance: 1.0, // reference Constants.EpsilonDistance
        }
    }

    /// reference `addPosition`: a new sample is pushed only after `min_move` px of travel (or
    /// when the timestamp changes); a same-timestamp sample updates in place.
    pub fn add_position(&mut self, position: f64, time: f64) {
        if let Some(p1) = self.position1.as_mut() {
            if p1.time == time {
                p1.position = position;
                return;
            }
            if (p1.position - position).abs() < self.min_move {
                return;
            }
        }
        self.position4 = self.position3;
        self.position3 = self.position2;
        self.position2 = self.position1;
        self.position1 = Some(TimeAndPosition { time, position });
    }

    /// reference `start`: freeze the release speed; a no-op when the samples cannot sustain a
    /// coast (too few, too stale, or below `min_speed`).
    pub fn start(&mut self, position: f64, time: f64) {
        let (Some(p1), Some(p2)) = (self.position1, self.position2) else {
            return;
        };
        if time - p1.time > MAX_START_DELAY_MS {
            return;
        }

        // Distance-weighted average speed; a segment counts only in the drag's release direction.
        let speed1 = speed_px_per_msec(p1, p2, self.max_speed);
        let mut speeds = vec![speed1];
        let mut distances = vec![p1.position - p2.position];
        let mut total_distance = distances[0];
        if let Some(p3) = self.position3 {
            let speed2 = speed_px_per_msec(p2, p3, self.max_speed);
            if speed2.signum() == speed1.signum() {
                speeds.push(speed2);
                distances.push(p2.position - p3.position);
                total_distance += distances[1];
                if let Some(p4) = self.position4 {
                    let speed3 = speed_px_per_msec(p3, p4, self.max_speed);
                    if speed3.signum() == speed1.signum() {
                        speeds.push(speed3);
                        distances.push(p3.position - p4.position);
                        total_distance += distances[2];
                    }
                }
            }
        }

        let mut result_speed = 0.0;
        for (speed, distance) in speeds.iter().zip(distances.iter()) {
            result_speed += distance / total_distance * speed;
        }
        if result_speed.abs() < self.min_speed {
            return;
        }

        self.animation_start_position = Some(TimeAndPosition { position, time });
        self.speed_px_per_msec = result_speed;
        self.duration_msecs = duration_msec(
            result_speed.abs(),
            self.dumping_coeff,
            self.epsilon_distance,
        );
    }

    /// Move every retained sample and the engaged coast anchor by `delta`. A data
    /// synchronization that rebases the scroll position (history insert, retention trim) calls
    /// this so an in-flight coast continues over the same content; speeds are unchanged.
    pub fn shift_positions(&mut self, delta: f64) {
        for sample in [
            &mut self.position1,
            &mut self.position2,
            &mut self.position3,
            &mut self.position4,
            &mut self.animation_start_position,
        ]
        .into_iter()
        .flatten()
        {
            sample.position += delta;
        }
    }

    /// reference `getPosition`. Only meaningful while `!finished(time)`.
    pub fn position(&self, time: f64) -> f64 {
        let Some(start) = self.animation_start_position else {
            return 0.0;
        };
        let duration_msecs = time - start.time;
        start.position
            + self.speed_px_per_msec * (self.dumping_coeff.powf(duration_msecs) - 1.0)
                / self.dumping_coeff.ln()
    }

    /// reference `finished` (also true when `start` never engaged).
    pub fn finished(&self, time: f64) -> bool {
        let Some(start) = self.animation_start_position else {
            return true;
        };
        (time - start.time).min(self.duration_msecs) == self.duration_msecs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anim() -> KineticAnimation {
        // reference KineticScrollConstants defaults in the px domain.
        KineticAnimation::new(0.2, 7.0, 0.997, 15.0)
    }

    #[test]
    fn no_coast_without_two_samples() {
        let mut a = anim();
        a.add_position(100.0, 1000.0);
        a.start(100.0, 1010.0);
        assert!(a.finished(1010.0));
    }

    #[test]
    fn no_coast_when_release_lags_the_last_sample() {
        let mut a = anim();
        a.add_position(100.0, 1000.0);
        a.add_position(140.0, 1020.0);
        // release more than MaxStartDelay after the last sample
        a.start(140.0, 1020.0 + 51.0);
        assert!(a.finished(1100.0));
    }

    #[test]
    fn no_coast_below_min_speed() {
        let mut a = anim();
        a.add_position(100.0, 1000.0);
        a.add_position(101.0, 1100.0); // 0.01 px/ms
        a.start(101.0, 1100.0);
        assert!(a.finished(1100.0));
    }

    #[test]
    fn coast_engages_and_decays_to_epsilon() {
        let mut a = anim();
        a.add_position(0.0, 1000.0);
        a.add_position(-100.0, 1050.0); // -2 px/ms
        a.start(-100.0, 1050.0);
        assert!(!a.finished(1050.0));
        // position formula: start + speed * (c^dt - 1) / ln(c)
        let expected = -100.0 + -2.0 * (0.997f64.powf(100.0) - 1.0) / 0.997f64.ln();
        assert!((a.position(1150.0) - expected).abs() < 1e-9);
        // the coast eventually finishes and stops moving
        assert!(a.finished(100_000.0));
    }

    #[test]
    fn opposite_direction_segments_do_not_count() {
        let mut a = anim();
        a.add_position(0.0, 1000.0);
        a.add_position(-100.0, 1050.0);
        a.add_position(-30.0, 1100.0); // reversed segment: dropped from the average
        a.start(-30.0, 1100.0);
        // speed = only the p1..p2 segment: (+70)/50 = 1.4 px/ms
        let expected = -30.0 + 1.4 * (0.997f64.powf(50.0) - 1.0) / 0.997f64.ln();
        assert!((a.position(1150.0) - expected).abs() < 1e-9);
    }

    #[test]
    fn segment_speed_is_capped_at_max_speed() {
        let mut a = anim();
        a.add_position(0.0, 1000.0);
        a.add_position(-1000.0, 1010.0); // raw 100 px/ms -> capped to 7
        a.start(-1000.0, 1010.0);
        let expected = -1000.0 + -7.0 * (0.997f64.powf(10.0) - 1.0) / 0.997f64.ln();
        assert!((a.position(1020.0) - expected).abs() < 1e-9);
    }

    #[test]
    fn min_move_gates_sampling() {
        let mut a = anim();
        a.add_position(100.0, 1000.0);
        a.add_position(105.0, 1010.0); // below min_move: ignored
        a.add_position(120.0, 1020.0); // 20 px from p1: kept
        a.start(120.0, 1020.0);
        // one valid segment of 20 px over 20 ms = 1 px/ms
        let expected = 120.0 + 1.0 * (0.997f64.powf(20.0) - 1.0) / 0.997f64.ln();
        assert!((a.position(1040.0) - expected).abs() < 1e-9);
    }
}
