//! Price-by-volume distribution from OHLCV bars. Volume is distributed uniformly over
//! each bar's low/high interval; this is an OHLCV estimate, not tick-level order flow.

pub const MAX_VOLUME_PROFILE_ROWS: usize = 512;

#[derive(Clone, Copy, Debug)]
pub struct ProfileBar {
    pub open: f64,
    pub low: f64,
    pub high: f64,
    pub close: f64,
    pub volume: f64,
}

impl ProfileBar {
    fn valid(self) -> bool {
        self.open.is_finite()
            && self.low.is_finite()
            && self.high.is_finite()
            && self.close.is_finite()
            && self.high >= self.low
            && self.volume.is_finite()
            && self.volume > 0.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProfileRow {
    pub low: f64,
    pub high: f64,
    pub volume: f64,
    pub up_volume: f64,
    pub down_volume: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VolumeProfile {
    pub rows: Vec<ProfileRow>,
    pub total_volume: f64,
    pub bar_count: usize,
    pub poc_index: Option<usize>,
    pub value_area_low_index: Option<usize>,
    pub value_area_high_index: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DevelopingProfilePoint {
    pub timestamp_micros: i64,
    pub poc_index: usize,
    pub value_area_low_index: usize,
    pub value_area_high_index: usize,
}

/// Incremental value-area path on the final profile's fixed price grid. The grid is held
/// constant so historical points remain comparable after a session's range grows. Work is
/// O(bars + sampled_points × rows + bar/bin overlaps), with a caller-owned sample budget.
pub fn volume_profile_developing(
    bars: &[(i64, ProfileBar)],
    final_rows: &[ProfileRow],
    value_area_percent: f64,
    sample_limit: usize,
) -> Result<Vec<DevelopingProfilePoint>, &'static str> {
    if final_rows.is_empty()
        || sample_limit == 0
        || !(0.0 < value_area_percent && value_area_percent <= 100.0)
    {
        return Ok(Vec::new());
    }
    let row_count = final_rows.len();
    let low = final_rows[0].low;
    let step = final_rows[0].high - low;
    if !step.is_finite() || step <= 0.0 {
        return Err("invalid volume-profile grid");
    }
    let valid_count = bars.iter().filter(|(_, bar)| bar.valid()).count();
    let stride = if sample_limit < 2 || valid_count < 2 {
        usize::MAX
    } else {
        (valid_count - 1).div_ceil(sample_limit - 1)
    };
    let mut volumes = vec![0.0; row_count];
    let mut total_volume = 0.0;
    let mut output = Vec::with_capacity(valid_count.min(sample_limit));
    let bin = |price: f64| (((price - low) / step).floor() as usize).min(row_count - 1);
    for (index, &(timestamp_micros, bar)) in bars.iter().filter(|(_, bar)| bar.valid()).enumerate()
    {
        let first = bin(bar.low);
        let last = bin(bar.high);
        if first == last || bar.low == bar.high {
            volumes[first] += bar.volume;
        } else {
            let span = bar.high - bar.low;
            volumes[first] += bar.volume * ((low + (first + 1) as f64 * step - bar.low) / span);
            volumes[last] += bar.volume * ((bar.high - (low + last as f64 * step)) / span);
            let full = bar.volume * (step / span);
            for volume in volumes.iter_mut().take(last).skip(first + 1) {
                *volume += full;
            }
        }
        total_volume += bar.volume;
        if !total_volume.is_finite() {
            return Err("volume-profile volume overflow");
        }
        if index + 1 != valid_count && (stride == usize::MAX || index % stride != 0) {
            continue;
        }
        if volumes.iter().any(|volume| !volume.is_finite()) {
            return Err("volume-profile bin overflow");
        }
        let mut poc = 0;
        for candidate in 1..row_count {
            if volumes[candidate] > volumes[poc] {
                poc = candidate;
            }
        }
        let target = total_volume * value_area_percent / 100.0;
        let (mut lower, mut upper, mut included) = (poc, poc, volumes[poc]);
        while included < target && (lower > 0 || upper + 1 < row_count) {
            if upper + 1 == row_count || (lower > 0 && volumes[lower - 1] >= volumes[upper + 1]) {
                lower -= 1;
                included += volumes[lower];
            } else {
                upper += 1;
                included += volumes[upper];
            }
        }
        output.push(DevelopingProfilePoint {
            timestamp_micros,
            poc_index: poc,
            value_area_low_index: lower,
            value_area_high_index: upper,
        });
    }
    Ok(output)
}

/// Two passes over the input and O(rows) storage/work beyond those passes. Full-bin
/// contributions use a difference array, avoiding a bars × rows inner loop.
/// POC ties choose the lowest price; value-area expansion chooses the larger adjacent
/// row, choosing the lower row on ties, until it reaches the requested volume fraction.
pub fn volume_profile(
    bars: impl Iterator<Item = ProfileBar> + Clone,
    row_count: usize,
    value_area_percent: f64,
    minimum_span: f64,
) -> Result<VolumeProfile, &'static str> {
    if !(1..=MAX_VOLUME_PROFILE_ROWS).contains(&row_count)
        || !value_area_percent.is_finite()
        || !(0.0..=100.0).contains(&value_area_percent)
        || value_area_percent == 0.0
        || !minimum_span.is_finite()
        || minimum_span <= 0.0
    {
        return Err("invalid volume-profile parameters");
    }
    let mut result = VolumeProfile::default();
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for bar in bars.clone().filter(|bar| bar.valid()) {
        low = low.min(bar.low);
        high = high.max(bar.high);
        result.total_volume += bar.volume;
        result.bar_count += 1;
    }
    if result.bar_count == 0 {
        return Ok(result);
    }
    if !result.total_volume.is_finite() {
        return Err("volume-profile volume overflow");
    }
    if low == high {
        low -= minimum_span * 0.5;
        high += minimum_span * 0.5;
    }
    let step = (high - low) / row_count as f64;
    if !step.is_finite() || step <= 0.0 || low + step == low {
        return Err("volume-profile price range cannot be represented");
    }
    let mut up_volumes = vec![0.0; row_count];
    let mut down_volumes = vec![0.0; row_count];
    let mut up_differences = vec![0.0; row_count + 1];
    let mut down_differences = vec![0.0; row_count + 1];
    let bin = |price: f64| (((price - low) / step).floor() as usize).min(row_count - 1);
    for bar in bars.filter(|bar| bar.valid()) {
        let (volumes, differences) = if bar.close >= bar.open {
            (&mut up_volumes, &mut up_differences)
        } else {
            (&mut down_volumes, &mut down_differences)
        };
        let first = bin(bar.low);
        let last = bin(bar.high);
        if first == last || bar.low == bar.high {
            volumes[first] += bar.volume;
            continue;
        }
        let span = bar.high - bar.low;
        // Divide the price overlap before multiplying by volume to avoid density overflow.
        volumes[first] += bar.volume * ((low + (first + 1) as f64 * step - bar.low) / span);
        volumes[last] += bar.volume * ((bar.high - (low + last as f64 * step)) / span);
        if last > first + 1 {
            let full = bar.volume * (step / span);
            differences[first + 1] += full;
            differences[last] -= full;
        }
    }
    let mut up_running = 0.0;
    let mut down_running = 0.0;
    let mut volumes = Vec::with_capacity(row_count);
    for index in 0..row_count {
        up_running += up_differences[index];
        down_running += down_differences[index];
        up_volumes[index] = (up_volumes[index] + up_running).max(0.0);
        down_volumes[index] = (down_volumes[index] + down_running).max(0.0);
        let volume = up_volumes[index] + down_volumes[index];
        if !volume.is_finite() {
            return Err("volume-profile bin overflow");
        }
        volumes.push(volume);
        result.rows.push(ProfileRow {
            low: low + index as f64 * step,
            high: if index + 1 == row_count {
                high
            } else {
                low + (index + 1) as f64 * step
            },
            volume,
            up_volume: up_volumes[index],
            down_volume: down_volumes[index],
        });
    }
    let mut poc = 0;
    for index in 1..row_count {
        if volumes[index] > volumes[poc] {
            poc = index;
        }
    }
    let (mut lower, mut upper) = (poc, poc);
    let mut included = volumes[poc];
    let target = result.total_volume * (value_area_percent / 100.0);
    while included < target && (lower > 0 || upper + 1 < row_count) {
        if upper + 1 == row_count || (lower > 0 && volumes[lower - 1] >= volumes[upper + 1]) {
            lower -= 1;
            included += volumes[lower];
        } else {
            upper += 1;
            included += volumes[upper];
        }
    }
    result.poc_index = Some(poc);
    result.value_area_low_index = Some(lower);
    result.value_area_high_index = Some(upper);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_overlap_conserves_volume_and_value_area_is_contiguous() {
        let bars = [
            ProfileBar {
                open: 0.0,
                low: 0.0,
                high: 4.0,
                close: 1.0,
                volume: 40.0,
            },
            ProfileBar {
                open: 2.0,
                low: 1.0,
                high: 2.0,
                close: 1.0,
                volume: 30.0,
            },
        ];
        let profile = volume_profile(bars.into_iter(), 4, 70.0, 0.01).unwrap();
        assert_eq!(
            profile
                .rows
                .iter()
                .map(|row| row.volume)
                .collect::<Vec<_>>(),
            [10.0, 40.0, 10.0, 10.0]
        );
        assert_eq!(profile.total_volume, 70.0);
        assert_eq!(profile.rows[1].up_volume, 10.0);
        assert_eq!(profile.rows[1].down_volume, 30.0);
        assert_eq!(profile.poc_index, Some(1));
        assert_eq!(
            (profile.value_area_low_index, profile.value_area_high_index),
            (Some(0), Some(1))
        );
    }

    #[test]
    fn developing_path_matches_final_distribution_and_samples_endpoints() {
        let bars = [
            (
                0,
                ProfileBar {
                    open: 1.0,
                    low: 0.0,
                    high: 2.0,
                    close: 2.0,
                    volume: 10.0,
                },
            ),
            (
                1,
                ProfileBar {
                    open: 2.0,
                    low: 1.0,
                    high: 3.0,
                    close: 1.0,
                    volume: 20.0,
                },
            ),
            (
                2,
                ProfileBar {
                    open: 1.0,
                    low: 0.0,
                    high: 4.0,
                    close: 4.0,
                    volume: 30.0,
                },
            ),
        ];
        let final_profile = volume_profile(bars.iter().map(|(_, bar)| *bar), 4, 70.0, 1.0).unwrap();
        let path = volume_profile_developing(&bars, &final_profile.rows, 70.0, 2).unwrap();
        assert_eq!(path.len(), 2);
        assert_eq!(path[0].timestamp_micros, 0);
        assert_eq!(path[1].timestamp_micros, 2);
        assert_eq!(path[1].poc_index, final_profile.poc_index.unwrap());
        assert_eq!(
            path[1].value_area_low_index,
            final_profile.value_area_low_index.unwrap()
        );
        assert_eq!(
            path[1].value_area_high_index,
            final_profile.value_area_high_index.unwrap()
        );
    }

    #[test]
    fn developing_last_value_matches_final_profile_across_mixed_bars() {
        let mut seed = 17_u64;
        for _fixture in 0..24 {
            let bars = (0..64)
                .map(|timestamp_micros| {
                    seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                    let low = ((seed >> 32) % 80) as f64 * 0.25;
                    seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                    let high = low + (((seed >> 32) % 12) + 1) as f64 * 0.25;
                    seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                    let volume = ((seed >> 32) % 50 + 1) as f64;
                    (
                        timestamp_micros,
                        ProfileBar {
                            open: low,
                            low,
                            high,
                            close: high,
                            volume,
                        },
                    )
                })
                .collect::<Vec<_>>();
            let final_profile =
                volume_profile(bars.iter().map(|(_, bar)| *bar), 32, 70.0, 0.25).unwrap();
            let path = volume_profile_developing(&bars, &final_profile.rows, 70.0, 64).unwrap();
            let last = path.last().unwrap();
            assert_eq!(last.poc_index, final_profile.poc_index.unwrap());
            assert_eq!(
                last.value_area_low_index,
                final_profile.value_area_low_index.unwrap()
            );
            assert_eq!(
                last.value_area_high_index,
                final_profile.value_area_high_index.unwrap()
            );
        }
    }

    #[test]
    fn flat_missing_and_invalid_data_never_create_fictitious_volume() {
        let bars = [
            ProfileBar {
                open: 2.0,
                low: 2.0,
                high: 2.0,
                close: 2.0,
                volume: 7.0,
            },
            ProfileBar {
                open: 1.0,
                low: 0.0,
                high: 3.0,
                close: 2.0,
                volume: -1.0,
            },
            ProfileBar {
                open: 1.0,
                low: f64::NAN,
                high: 4.0,
                close: 2.0,
                volume: 5.0,
            },
        ];
        let profile = volume_profile(bars.into_iter(), 8, 100.0, 0.01).unwrap();
        assert_eq!(profile.bar_count, 1);
        assert_eq!(profile.rows.iter().map(|row| row.volume).sum::<f64>(), 7.0);
        assert!(
            volume_profile(std::iter::empty(), 512, 70.0, 0.01)
                .unwrap()
                .rows
                .is_empty()
        );
        assert!(volume_profile(std::iter::empty(), 513, 70.0, 0.01).is_err());
        assert!(volume_profile(bars.into_iter(), 4, f64::NAN, 0.01).is_err());
    }

    #[test]
    fn overflow_is_reported_and_ties_are_deterministic() {
        let bars = [ProfileBar {
            open: 0.0,
            low: 0.0,
            high: 4.0,
            close: 1.0,
            volume: 40.0,
        }];
        assert_eq!(
            volume_profile(bars.into_iter(), 4, 70.0, 0.01)
                .unwrap()
                .poc_index,
            Some(0)
        );
        let huge = [ProfileBar {
            open: 0.0,
            low: 0.0,
            high: 1.0,
            close: 1.0,
            volume: f64::MAX,
        }; 2];
        assert!(volume_profile(huge.into_iter(), 4, 70.0, 0.01).is_err());
    }
}
