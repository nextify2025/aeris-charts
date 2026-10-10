//! Engine-owned typing session for every drawing that paints its own text: the text tool, the
//! multi-line text blocks of the text annotations (note, comment, callout, price note, anchored
//! text) and of the fork-form signpost plate, arrow-mark text, and price-label bubble, the
//! one-line run labels of trend lines and every other line, channel, Fibonacci, pitchfork,
//! pattern, and shape tool, and the multi-line text boxes of the family tools that own their
//! text.
//!
//! The session owns the product rules every host shares: live text, caret and selection,
//! Enter/Escape semantics, and the empty lifecycle (an emptied text tool or text annotation is
//! removed; every other drawing keeps its emptied label or box). Live text is written straight to
//! the drawing, so typing records no undo step and no sync revision: commit records the whole edit
//! as one `Update` and cancel restores the text and revision the session began from. Hosts only
//! forward input. A browser host keeps its native editable surface for IME, clipboard, and
//! accessibility and mirrors it through [`ChartEngine::set_drawing_text_edit`]; native hosts use
//! the typing API and ask the engine to paint the caret in the canonical frame.

use crate::drawings::{DrawingId, DrawingKind, collapse_line_breaks};
use crate::{ChartEngine, MAX_DRAWING_TEXT_BYTES};

/// Editing keys a host forwards while a drawing text session is open. Movement keys extend the
/// selection when the host passes `extend_selection` (Shift); word variants follow the host's
/// word modifier (Ctrl on Windows/Linux, Alt/Option on macOS). Home and End act on the whole
/// text; native line navigation inside a family text box is not part of this contract yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawingTextEditKey {
    Backspace,
    Delete,
    DeleteWordBackward,
    DeleteWordForward,
    Left,
    Right,
    WordLeft,
    WordRight,
    Home,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DrawingTextEditSession {
    pub(crate) id: DrawingId,
    /// The drawing's text and revision when the session began: what cancel restores and what
    /// commit records as the undo step's `before`.
    original: String,
    original_revision: u64,
    /// Always equal to the drawing's `text` after every apply.
    pub(crate) text: String,
    /// Caret position in `char`s from the start of `text`.
    pub(crate) caret: usize,
    /// Selection anchor in `char`s; the selection spans anchor..caret. Like the browser editor,
    /// the selection is not painted, but typing, deletion, copy, and cut all honor it.
    anchor: Option<usize>,
    /// Native hosts have no editable surface of their own, so the frame paints the caret.
    pub(crate) paint_caret: bool,
    /// The painted caret's blink phase: shown, and the host-clock time of its next toggle.
    /// Every edit or caret move shows it and restarts the cycle (`None` until the next tick
    /// supplies the clock).
    pub(crate) caret_shown: bool,
    pub(crate) caret_toggle_ms: Option<f64>,
}

/// Half of the painted caret's blink cycle (the common platform 1.06 s rate).
pub(crate) const CARET_BLINK_MS: f64 = 530.0;

/// The engine's text rules for one session. A run label stays on one line (a run of line breaks
/// becomes one space) and a text box or block keeps its line breaks (`\r\n` and `\r` become `\n`);
/// every other control character, a tab included, becomes a space.
fn sanitize(text: &str, multiline: bool) -> String {
    if !multiline {
        return collapse_line_breaks(text)
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\n' => out.push('\n'),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// Start of the word before `caret`: skip whitespace leftwards, then the word itself.
fn word_left(chars: &[char], caret: usize) -> usize {
    let mut index = caret.min(chars.len());
    while index > 0 && chars[index - 1].is_whitespace() {
        index -= 1;
    }
    while index > 0 && !chars[index - 1].is_whitespace() {
        index -= 1;
    }
    index
}

/// Start of the next word after `caret`: skip the current word, then whitespace.
fn word_right(chars: &[char], caret: usize) -> usize {
    let mut index = caret.min(chars.len());
    while index < chars.len() && !chars[index].is_whitespace() {
        index += 1;
    }
    while index < chars.len() && chars[index].is_whitespace() {
        index += 1;
    }
    index
}

/// Whether an emptied `drawing` is removed when its session ends: the text tool and the text
/// annotations ([`crate::Drawing::text_annotation`]) are their text, so an empty one has nothing
/// left to show. A fork-form signpost keeps its emptied plate, as the fork did.
fn removes_when_empty(drawing: &crate::Drawing) -> bool {
    drawing.kind == DrawingKind::Text || drawing.text_annotation()
}

fn byte_index(text: &str, caret: usize) -> usize {
    text.char_indices()
        .nth(caret)
        .map_or(text.len(), |(index, _)| index)
}

impl ChartEngine {
    /// Whether `id`'s text may span lines (a family text box, a text annotation's block, or a
    /// fork-form annotation's box) rather than a one-line run.
    fn drawing_text_edit_multiline(&self, id: DrawingId) -> bool {
        self.drawing(id).is_some_and(|drawing| {
            !drawing.kind.paints_generic_text() || drawing.paints_text_block()
        })
    }

    /// Open typing mode on any drawing that paints its own text ([`ChartEngine::drawing_text_editable`]:
    /// unlocked, visible, shown on the interval, with a layout inside its pane's plot).
    /// `paint_caret` asks the frame to draw the caret (native hosts); a browser host that paints
    /// its own caret passes `false`. Beginning on the drawing already being edited only refreshes
    /// `paint_caret`. A refused begin (unknown, locked, hidden, not text-bearing, anchors that
    /// cannot convert, or text wholly outside the plot) leaves any open session alone; otherwise
    /// a session open on another drawing is committed first.
    pub fn begin_drawing_text_edit(&mut self, id: DrawingId, paint_caret: bool) -> bool {
        if let Some(session) = self
            .drawing_text_edit
            .as_mut()
            .filter(|session| session.id == id)
        {
            if session.paint_caret != paint_caret {
                session.paint_caret = paint_caret;
                self.invalidate_frame_drawings();
            }
            return true;
        }
        if !self.drawing_text_editable(id) {
            return false;
        }
        self.commit_drawing_text_edit();
        let Some(drawing) = self.drawing(id) else {
            return false;
        };
        let text = drawing.text.clone();
        let revision = drawing.revision;
        self.drawing_text_edit = Some(DrawingTextEditSession {
            id,
            original: text.clone(),
            original_revision: revision,
            caret: text.chars().count(),
            text,
            anchor: None,
            paint_caret,
            caret_shown: true,
            caret_toggle_ms: None,
        });
        // The open session gives an empty label its caret slot, which changes its culling pad.
        self.update_drawing_runtime(id);
        self.invalidate_frame_drawings();
        true
    }

    /// The open session as `(drawing, text, caret)`, with the caret in `char`s.
    pub fn drawing_text_edit(&self) -> Option<(DrawingId, &str, usize)> {
        self.drawing_text_edit
            .as_ref()
            .map(|session| (session.id, session.text.as_str(), session.caret))
    }

    /// Advance the engine-painted caret's blink on the host clock (from [`Self::input_tick`]).
    /// The first tick after an edit starts the cycle; returns whether the caret toggled.
    pub(crate) fn tick_drawing_text_caret(&mut self, now_ms: f64) -> bool {
        let Some(session) = self
            .drawing_text_edit
            .as_mut()
            .filter(|session| session.paint_caret)
        else {
            return false;
        };
        match session.caret_toggle_ms {
            None => {
                session.caret_toggle_ms = Some(now_ms + CARET_BLINK_MS);
                false
            }
            Some(deadline) if now_ms >= deadline => {
                // A stalled host resumes on the cycle instead of replaying missed toggles.
                let missed = ((now_ms - deadline) / CARET_BLINK_MS).floor();
                if missed % 2.0 == 0.0 {
                    session.caret_shown = !session.caret_shown;
                }
                session.caret_toggle_ms = Some(deadline + (missed + 1.0) * CARET_BLINK_MS);
                self.invalidate_frame_drawings();
                true
            }
            Some(_) => false,
        }
    }

    /// When the engine-painted caret next toggles, while one is blinking.
    pub(crate) fn drawing_text_caret_deadline_ms(&self) -> Option<f64> {
        self.drawing_text_edit
            .as_ref()
            .filter(|session| session.paint_caret)
            .and_then(|session| session.caret_toggle_ms)
    }

    /// A host with a native text input surface paints its own caret over the shared label.
    pub fn set_drawing_text_edit_paint_caret(&mut self, paint_caret: bool) {
        if let Some(session) = self.drawing_text_edit.as_mut()
            && session.paint_caret != paint_caret
        {
            session.paint_caret = paint_caret;
            self.invalidate_frame_drawings();
        }
    }

    /// The selected `char` range, if any.
    fn drawing_text_edit_range(session: &DrawingTextEditSession) -> Option<(usize, usize)> {
        session
            .anchor
            .filter(|&anchor| anchor != session.caret)
            .map(|anchor| (anchor.min(session.caret), anchor.max(session.caret)))
    }

    /// The selected text of the open session (for host copy/cut), if any.
    pub fn drawing_text_edit_selection(&self) -> Option<&str> {
        let session = self.drawing_text_edit.as_ref()?;
        let (start, end) = Self::drawing_text_edit_range(session)?;
        let text = session.text.as_str();
        Some(&text[byte_index(text, start)..byte_index(text, end)])
    }

    /// Select the whole label (Ctrl/Cmd+A).
    pub fn drawing_text_edit_select_all(&mut self) -> bool {
        let Some(session) = self.drawing_text_edit.as_mut() else {
            return false;
        };
        let len = session.text.chars().count();
        if session.anchor == Some(0) && session.caret == len {
            return false;
        }
        session.anchor = Some(0);
        session.caret = len;
        // The caret moved: it shows solid and restarts the blink.
        session.caret_shown = true;
        session.caret_toggle_ms = None;
        self.invalidate_frame_drawings();
        true
    }

    /// Insert committed text input at the caret, replacing any selection. Input that would exceed
    /// the text bound ([`MAX_DRAWING_TEXT_BYTES`]) is rejected whole so a paste never lands
    /// half-applied.
    pub fn drawing_text_edit_insert(&mut self, input: &str) -> bool {
        let Some(session) = self.drawing_text_edit.as_ref() else {
            return false;
        };
        let input = sanitize(input, self.drawing_text_edit_multiline(session.id));
        let (start, end) =
            Self::drawing_text_edit_range(session).unwrap_or((session.caret, session.caret));
        let mut text = session.text.clone();
        let (from, to) = (byte_index(&text, start), byte_index(&text, end));
        if input.is_empty() || text.len() - (to - from) + input.len() > MAX_DRAWING_TEXT_BYTES {
            return false;
        }
        text.replace_range(from..to, &input);
        let caret = start + input.chars().count();
        self.apply_drawing_text_edit_with_anchor(text, caret, None)
    }

    /// Apply one editing key; `extend_selection` (Shift) turns movement into selection.
    /// Deletion removes the selection when there is one. Returns whether anything changed.
    pub fn drawing_text_edit_key(
        &mut self,
        key: DrawingTextEditKey,
        extend_selection: bool,
    ) -> bool {
        let Some(session) = self.drawing_text_edit.as_ref() else {
            return false;
        };
        let chars: Vec<char> = session.text.chars().collect();
        let len = chars.len();
        let caret = session.caret.min(len);
        let range = Self::drawing_text_edit_range(session);
        let deletion = match key {
            DrawingTextEditKey::Backspace => Some((caret.saturating_sub(1), caret)),
            DrawingTextEditKey::Delete => Some((caret, (caret + 1).min(len))),
            DrawingTextEditKey::DeleteWordBackward => Some((word_left(&chars, caret), caret)),
            DrawingTextEditKey::DeleteWordForward => Some((caret, word_right(&chars, caret))),
            _ => None,
        };
        let (text, next_caret, anchor) = if let Some(span) = deletion {
            let (start, end) = range.unwrap_or(span);
            if start == end {
                return false;
            }
            let mut text = session.text.clone();
            text.replace_range(byte_index(&text, start)..byte_index(&text, end), "");
            (text, start, None)
        } else {
            let target = match key {
                DrawingTextEditKey::Left if !extend_selection && range.is_some() => {
                    range.map_or(caret, |(start, _)| start)
                }
                DrawingTextEditKey::Right if !extend_selection && range.is_some() => {
                    range.map_or(caret, |(_, end)| end)
                }
                DrawingTextEditKey::Left => caret.saturating_sub(1),
                DrawingTextEditKey::Right => (caret + 1).min(len),
                DrawingTextEditKey::WordLeft => word_left(&chars, caret),
                DrawingTextEditKey::WordRight => word_right(&chars, caret),
                DrawingTextEditKey::Home => 0,
                _ => len,
            };
            let anchor = extend_selection.then(|| session.anchor.unwrap_or(caret));
            (session.text.clone(), target, anchor)
        };
        if text == session.text && next_caret == session.caret && anchor == session.anchor {
            return false;
        }
        self.apply_drawing_text_edit_with_anchor(text, next_caret, anchor)
    }

    /// Place the caret at the character boundary nearest a media-px pointer (the browser
    /// editor's click-to-place behavior for native hosts). The pointer is read in the label's own
    /// frame from the shared editor layout ([`ChartEngine::drawing_text_edit_layout`]): rotated
    /// back by the run's angle, and in a family text box matched to the nearest line first.
    pub fn drawing_text_edit_caret_at(&mut self, x: f64, y: f64) -> bool {
        let Some(session) = self.drawing_text_edit.as_ref() else {
            return false;
        };
        let Some(layout) = self.drawing_text_edit_layout(session.id) else {
            return false;
        };
        let (dx, dy) = (x - layout.x, y - layout.y);
        let (sin, cos) = layout.angle.sin_cos();
        let (local_x, local_y) = (dx * cos + dy * sin, -dx * sin + dy * cos);
        let lines: Vec<&str> = session.text.split('\n').collect();
        let row = if layout.multiline {
            ((local_y / layout.line_height).round().max(0.0) as usize).min(lines.len() - 1)
        } else {
            0
        };
        let line = lines[row];
        let measure = |prefix: &str| {
            self.measure_text_run(
                prefix,
                layout.size,
                &layout.font_family,
                layout.weight,
                layout.italic,
            )
        };
        // The width of a prefix never shrinks as the prefix grows, so the boundary nearest the
        // pointer is found by bisection (a handful of measures) instead of measuring every
        // prefix, which is quadratic on the 64 KiB label bound.
        let boundaries: Vec<usize> = line
            .char_indices()
            .map(|(offset, _)| offset)
            .chain(std::iter::once(line.len()))
            .collect();
        let width_at = |index: usize| {
            if index == 0 {
                0.0
            } else {
                measure(&line[..boundaries[index]])
            }
        };
        let (mut low, mut high) = (0, boundaries.len() - 1);
        while low < high {
            let middle = (low + high) / 2;
            if width_at(middle) < local_x {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        // The first boundary at or past the pointer and the one before it; a tie takes the
        // earlier boundary.
        let mut best = (low, (width_at(low) - local_x).abs());
        if low > 0 {
            let before = (width_at(low - 1) - local_x).abs();
            if before <= best.1 {
                best = (low - 1, before);
            }
        }
        let before: usize = lines[..row]
            .iter()
            .map(|earlier| earlier.chars().count() + 1)
            .sum();
        let caret = before + best.0;
        if caret == session.caret && session.anchor.is_none() {
            return false;
        }
        let text = session.text.clone();
        self.apply_drawing_text_edit_with_anchor(text, caret, None)
    }

    /// Mirror a host-owned editable surface (browser IME/clipboard) into the session: the whole
    /// current value plus its caret in `char`s. A DOM value can never be refused, so a value over
    /// [`MAX_DRAWING_TEXT_BYTES`] is clamped at a character boundary (the caret follows). False
    /// while no session is open or once the drawing is gone.
    pub fn set_drawing_text_edit(&mut self, text: &str, caret: usize) -> bool {
        let Some(session) = self.drawing_text_edit.as_ref() else {
            return false;
        };
        let mut text = sanitize(text, self.drawing_text_edit_multiline(session.id));
        let mut end = text.len().min(MAX_DRAWING_TEXT_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        let caret = caret.min(text.chars().count());
        self.apply_drawing_text_edit_with_anchor(text, caret, None)
    }

    /// Enter / blur: keep the typed text, trimmed. A change records one `Update` undo step and one
    /// sync revision; an emptied text tool or text annotation (`Drawing::text_annotation`: the
    /// note, comment, callout, upstream-form signpost, anchored text, and fork-form price note) is
    /// then removed (so the history reads Update, Delete), while every other drawing keeps its
    /// emptied label or box. False while no session is open.
    pub fn commit_drawing_text_edit(&mut self) -> bool {
        let Some(session) = self.drawing_text_edit.take() else {
            return false;
        };
        let id = session.id;
        if let Some(index) = self.drawings.iter().position(|drawing| drawing.id == id) {
            let trimmed = self.drawings[index].text.trim().to_string();
            if trimmed != self.drawings[index].text {
                let drawing = &mut self.drawings[index];
                drawing.text = trimmed;
                drawing.revision = drawing.revision.saturating_add(1);
            }
            if self.drawings[index].text != session.original {
                let after = self.drawings[index].clone();
                let mut before = after.clone();
                before.text = session.original;
                before.revision = session.original_revision;
                self.record_drawing_update(before, after);
            }
            self.update_drawing_runtime(id);
            let drawing = &self.drawings[index];
            if removes_when_empty(drawing) && drawing.text.is_empty() {
                self.remove_drawing(id);
            }
        }
        self.invalidate_frame_drawings();
        true
    }

    /// Escape: restore the text and revision from before the session, recording nothing. A text
    /// tool or text annotation that began empty (a fresh placement) is removed. False while no
    /// session is open.
    pub fn cancel_drawing_text_edit(&mut self) -> bool {
        let Some(session) = self.drawing_text_edit.take() else {
            return false;
        };
        let id = session.id;
        let mut remove = false;
        if let Some(drawing) = self.drawings.iter_mut().find(|drawing| drawing.id == id) {
            remove = removes_when_empty(drawing) && session.original.trim().is_empty();
            drawing.text = session.original;
            drawing.revision = session.original_revision;
        }
        self.update_drawing_runtime(id);
        if remove {
            self.remove_drawing(id);
        }
        self.invalidate_frame_drawings();
        true
    }

    /// Store the session's new text, caret, and selection, and write the text straight to the
    /// drawing (no history, no sync revision; the drawing's `revision` advances like any text
    /// change). A drawing that vanished underneath the session ends it.
    fn apply_drawing_text_edit_with_anchor(
        &mut self,
        text: String,
        caret: usize,
        anchor: Option<usize>,
    ) -> bool {
        let Some(session) = self.drawing_text_edit.as_mut() else {
            return false;
        };
        let (id, paint) = (session.id, session.paint_caret);
        let changed = session.text != text;
        session.text = text.clone();
        session.caret = caret;
        session.anchor = anchor;
        // Typing or moving the caret shows it solid and restarts the blink.
        session.caret_shown = true;
        session.caret_toggle_ms = None;
        let Some(drawing) = self.drawings.iter_mut().find(|drawing| drawing.id == id) else {
            self.drawing_text_edit = None;
            return false;
        };
        if drawing.text != text {
            drawing.text = text;
            drawing.revision = drawing.revision.saturating_add(1);
            self.update_drawing_runtime(id);
        }
        // A browser mirror that only moves its own caret repaints nothing.
        if changed || paint {
            self.invalidate_frame_drawings();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawings::DrawingPoint;
    use aeris_charts_render::draw_list::Prim;

    fn chart_with(kind: DrawingKind, text: &str) -> (ChartEngine, DrawingId) {
        let mut chart = ChartEngine::new(800.0, 400.0, 1.0);
        let times = (0..20)
            .map(|i| 1_700_000_000.0 + f64::from(i) * 60.0)
            .collect::<Vec<_>>();
        let values = vec![100.0; 20];
        let (high, low) = (vec![110.0; 20], vec![90.0; 20]);
        chart
            .set_series_data(0, &times, &values, &high, &low, &values)
            .expect("valid bars");
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        // The drawings sit inside the scaled bars: an editor opens only on text in view.
        chart.autoscale_visible();
        let points = match kind.anchor_count() {
            1 => vec![DrawingPoint {
                logical: 5.0,
                price: 100.0,
            }],
            _ => vec![
                DrawingPoint {
                    logical: 2.0,
                    price: 99.0,
                },
                DrawingPoint {
                    logical: 12.0,
                    price: 101.0,
                },
            ],
        };
        let id = chart.add_drawing(kind, 0, points, None).expect("drawing");
        if !text.is_empty() {
            let patch = serde_json::json!({ "text": text }).to_string();
            assert!(chart.drawing_apply_options(id, &patch));
        }
        // Beginning an edit needs a settled chart: the editor layout converts the anchors.
        chart.build_frame();
        assert!(
            chart.drawing_text_editable(id),
            "the fixture text is in view"
        );
        (chart, id)
    }

    fn text(chart: &ChartEngine, id: DrawingId) -> Option<String> {
        chart.drawing(id).map(|drawing| drawing.text.clone())
    }

    #[test]
    fn typing_edits_the_trend_label_live_at_the_caret() {
        let (mut chart, id) = chart_with(DrawingKind::TrendLine, "");
        assert!(chart.begin_drawing_text_edit(id, true));
        assert_eq!(chart.editing_drawing(), Some(id));
        assert!(chart.drawing_text_edit_insert("Brkoutx"));
        assert_eq!(text(&chart, id).as_deref(), Some("Brkoutx"));
        assert!(chart.drawing_text_edit_key(DrawingTextEditKey::Backspace, false));
        for _ in 0..4 {
            chart.drawing_text_edit_key(DrawingTextEditKey::Left, false);
        }
        assert!(chart.drawing_text_edit_insert("ea"));
        assert_eq!(chart.drawing_text_edit(), Some((id, "Breakout", 4)));
        chart.drawing_text_edit_key(DrawingTextEditKey::End, false);
        assert!(chart.drawing_text_edit_insert("\nnow"));
        assert!(chart.commit_drawing_text_edit());
        assert_eq!(text(&chart, id).as_deref(), Some("Breakout now"));
        assert_eq!(chart.editing_drawing(), None);
        assert!(chart.drawing_text_edit().is_none());
    }

    #[test]
    fn an_upstream_signpost_stays_one_line_and_the_other_boxes_keep_their_lines() {
        // Upstream's signpost box is one line (R9 approved multi-line for the note, comment,
        // callout, price note, and anchored text only): a pasted or typed line break becomes a
        // space and its editor is single-line.
        for (kind, multiline) in [
            (DrawingKind::Signpost, false),
            (DrawingKind::Note, true),
            (DrawingKind::Comment, true),
            (DrawingKind::Callout, true),
            (DrawingKind::PriceNote, true),
        ] {
            let (mut chart, id) = chart_with(kind, "");
            assert_eq!(
                chart
                    .drawing_text_edit_layout(id)
                    .map(|layout| layout.multiline),
                Some(multiline),
                "{kind:?}"
            );
            assert!(chart.begin_drawing_text_edit(id, false), "{kind:?}");
            assert!(chart.drawing_text_edit_insert("up\nnow"));
            assert!(chart.set_drawing_text_edit("up\nnow\r\nthen", 13));
            let expected = if multiline {
                "up\nnow\nthen"
            } else {
                "up now then"
            };
            assert_eq!(chart.drawing_text_edit().map(|edit| edit.1), Some(expected));
            assert!(chart.commit_drawing_text_edit());
            assert_eq!(chart.drawing(id).unwrap().text, expected, "{kind:?}");
        }
    }

    #[test]
    fn family_box_and_multi_line_annotation_carets_blink_with_the_session() {
        // Every engine-painted caret follows the blink phase: a family text box's (and a
        // fork-form box's, painted by the same parts path) and a multi-line box annotation's.
        for (kind, text) in [
            (DrawingKind::SimpleAnnotation, "Boxed"),
            (DrawingKind::Note, "first\nsecond"),
        ] {
            let (mut chart, id) = chart_with(kind, text);
            let rules = |chart: &mut ChartEngine| {
                chart.build_frame().panes[0]
                    .main
                    .iter()
                    .filter(|prim| {
                        matches!(prim, Prim::Rect { rect, .. } if rect.w == 1 && rect.h > 10)
                    })
                    .count()
            };
            let baseline = rules(&mut chart);
            assert!(chart.begin_drawing_text_edit(id, true), "{kind:?}");
            assert_eq!(rules(&mut chart), baseline + 1, "{kind:?} shown");
            assert!(!chart.input_tick(1_000.0));
            assert!(chart.input_tick(1_000.0 + CARET_BLINK_MS));
            assert_eq!(
                rules(&mut chart),
                baseline,
                "{kind:?} hidden in the off phase"
            );
            assert!(chart.input_tick(1_000.0 + 2.0 * CARET_BLINK_MS));
            assert_eq!(rules(&mut chart), baseline + 1, "{kind:?} shown again");
        }
    }

    #[test]
    fn the_painted_caret_blinks_on_the_host_clock_and_restarts_solid_on_edits() {
        let (mut chart, id) = chart_with(DrawingKind::Text, "Blink");
        // The caret: one crisp 1 px rule the height of the line box, over the rules the chart
        // paints without a session.
        let rules = |chart: &mut ChartEngine| {
            chart.build_frame().panes[0]
                .main
                .iter()
                .filter(
                    |prim| matches!(prim, Prim::Rect { rect, .. } if rect.w == 1 && rect.h > 10),
                )
                .count()
        };
        let baseline = rules(&mut chart);
        let caret_rects = |chart: &mut ChartEngine| rules(chart) - baseline;
        assert!(chart.begin_drawing_text_edit(id, true));
        assert_eq!(caret_rects(&mut chart), 1);
        // The first tick starts the cycle; the caret hides after half a period, then returns.
        assert!(!chart.input_tick(1_000.0));
        assert_eq!(
            chart.input_wake_deadline_ms(),
            Some(1_000.0 + CARET_BLINK_MS)
        );
        assert!(!chart.input_tick(1_000.0 + CARET_BLINK_MS - 1.0));
        assert!(chart.input_tick(1_000.0 + CARET_BLINK_MS));
        assert_eq!(caret_rects(&mut chart), 0, "hidden in the off phase");
        assert!(chart.input_tick(1_000.0 + 2.0 * CARET_BLINK_MS));
        assert_eq!(caret_rects(&mut chart), 1, "shown again");
        // A stalled host resumes in phase: three missed toggles leave it hidden.
        assert!(chart.input_tick(1_000.0 + 5.0 * CARET_BLINK_MS + 10.0));
        assert_eq!(caret_rects(&mut chart), 0);
        assert_eq!(
            chart.input_wake_deadline_ms(),
            Some(1_000.0 + 6.0 * CARET_BLINK_MS)
        );
        // Typing shows the caret solid and restarts the cycle from the next tick.
        assert!(chart.drawing_text_edit_insert("!"));
        assert_eq!(caret_rects(&mut chart), 1);
        assert_eq!(chart.input_wake_deadline_ms(), None);
        assert!(!chart.input_tick(5_000.0));
        assert_eq!(
            chart.input_wake_deadline_ms(),
            Some(5_000.0 + CARET_BLINK_MS)
        );
        // A host that paints its own caret (the browser) needs no blink wakes.
        assert!(chart.commit_drawing_text_edit());
        assert!(chart.begin_drawing_text_edit(id, false));
        assert!(!chart.input_tick(6_000.0));
        assert_eq!(chart.input_wake_deadline_ms(), None);
    }

    #[test]
    fn cancel_restores_and_empty_lifecycle_follows_the_drawing_kind() {
        let (mut chart, id) = chart_with(DrawingKind::TrendLine, "keep");
        assert!(chart.begin_drawing_text_edit(id, true));
        chart.drawing_text_edit_key(DrawingTextEditKey::Backspace, false);
        assert_eq!(text(&chart, id).as_deref(), Some("kee"));
        assert!(chart.cancel_drawing_text_edit());
        assert_eq!(text(&chart, id).as_deref(), Some("keep"));

        // Clearing a trend label keeps the line; clearing standalone text removes the drawing.
        assert!(chart.begin_drawing_text_edit(id, true));
        assert!(chart.set_drawing_text_edit("  ", 2));
        assert!(chart.commit_drawing_text_edit());
        assert_eq!(text(&chart, id).as_deref(), Some(""));

        let (mut chart, note) = chart_with(DrawingKind::Text, "");
        assert!(chart.begin_drawing_text_edit(note, false));
        assert!(chart.cancel_drawing_text_edit());
        assert!(chart.drawing(note).is_none());
    }

    #[test]
    fn selection_extends_replaces_deletes_and_collapses_like_a_text_field() {
        let (mut chart, id) = chart_with(DrawingKind::TrendLine, "buy the dip");
        assert!(chart.begin_drawing_text_edit(id, true));
        // Shift+word-left selects "dip"; typing replaces it.
        assert!(chart.drawing_text_edit_key(DrawingTextEditKey::WordLeft, true));
        assert_eq!(chart.drawing_text_edit_selection(), Some("dip"));
        assert!(chart.drawing_text_edit_insert("rip"));
        assert_eq!(chart.drawing_text_edit(), Some((id, "buy the rip", 11)));
        assert_eq!(chart.drawing_text_edit_selection(), None);

        // Ctrl+Backspace removes a word; select-all then Delete clears everything.
        assert!(chart.drawing_text_edit_key(DrawingTextEditKey::DeleteWordBackward, false));
        assert_eq!(chart.drawing_text_edit(), Some((id, "buy the ", 8)));
        assert!(chart.drawing_text_edit_select_all());
        assert_eq!(chart.drawing_text_edit_selection(), Some("buy the "));
        // A plain arrow collapses the selection to its edge without moving past it.
        assert!(chart.drawing_text_edit_key(DrawingTextEditKey::Left, false));
        assert_eq!(chart.drawing_text_edit(), Some((id, "buy the ", 0)));
        assert!(chart.drawing_text_edit_key(DrawingTextEditKey::End, true));
        assert!(chart.drawing_text_edit_key(DrawingTextEditKey::Delete, false));
        assert_eq!(chart.drawing_text_edit(), Some((id, "", 0)));
    }

    #[test]
    fn a_click_places_the_caret_at_the_nearest_character_boundary() {
        let (mut chart, id) = chart_with(DrawingKind::Text, "");
        chart.set_text_measure(Some(Box::new(|text, _, _, _, _| {
            text.chars().count() as f64 * 10.0
        })));
        assert!(chart.drawing_apply_options(id, r#"{"text":"abcd","text_h_align":"left"}"#));
        chart.build_frame();
        assert!(chart.begin_drawing_text_edit(id, true));
        let (x, y, _) = chart.drawing_text_transform(id).unwrap();
        assert!(chart.drawing_text_edit_caret_at(x + 12.0, y));
        assert_eq!(chart.drawing_text_edit(), Some((id, "abcd", 1)));
        assert!(chart.drawing_text_edit_caret_at(x + 26.0, y));
        assert_eq!(chart.drawing_text_edit(), Some((id, "abcd", 3)));
        assert!(chart.drawing_text_edit_caret_at(x - 30.0, y));
        assert_eq!(chart.drawing_text_edit(), Some((id, "abcd", 0)));
    }

    #[test]
    fn click_placement_on_a_long_line_measures_a_logarithmic_number_of_prefixes() {
        // The text bound is 64 KiB, so a click inside a long label must not measure every prefix
        // (that is quadratic work on the input thread). Prefix widths grow with the prefix, so a
        // bisection finds the nearest boundary.
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = Arc::new(AtomicUsize::new(0));
        let (mut chart, id) = chart_with(DrawingKind::Text, "");
        let counter = Arc::clone(&calls);
        chart.set_text_measure(Some(Box::new(move |text, _, _, _, _| {
            counter.fetch_add(1, Ordering::Relaxed);
            text.chars().count() as f64 * 10.0
        })));
        let long = "a".repeat(20_000);
        assert!(
            chart.drawing_apply_options(
                id,
                &format!(r#"{{"text":"{long}","text_h_align":"left"}}"#)
            )
        );
        chart.build_frame();
        assert!(chart.begin_drawing_text_edit(id, true));
        let (x, y, _) = chart.drawing_text_transform(id).unwrap();
        calls.store(0, Ordering::Relaxed);
        // 123_457 px at 10 px per character is 12_345.7 characters in: nearest boundary 12_346.
        assert!(chart.drawing_text_edit_caret_at(x + 123_457.0, y));
        assert_eq!(chart.drawing_text_edit().map(|edit| edit.2), Some(12_346));
        let measured = calls.load(Ordering::Relaxed);
        assert!(measured <= 60, "{measured} prefix measures for one click");
        // Past either end the caret clamps to the boundary, and a tie takes the earlier one.
        assert!(chart.drawing_text_edit_caret_at(x + 1.0e9, y));
        assert_eq!(chart.drawing_text_edit().map(|edit| edit.2), Some(20_000));
        assert!(chart.drawing_text_edit_caret_at(x + 15.0, y));
        assert_eq!(chart.drawing_text_edit().map(|edit| edit.2), Some(1));
    }

    #[test]
    fn oversized_input_is_rejected_whole_and_multibyte_caret_is_char_based() {
        let (mut chart, id) = chart_with(DrawingKind::TrendLine, "");
        assert!(chart.begin_drawing_text_edit(id, true));
        assert!(!chart.drawing_text_edit_insert(&"x".repeat(MAX_DRAWING_TEXT_BYTES + 1)));
        assert_eq!(chart.drawing_text_edit(), Some((id, "", 0)));
        assert!(chart.drawing_text_edit_insert("€€"));
        assert!(chart.drawing_text_edit_key(DrawingTextEditKey::Backspace, false));
        assert_eq!(chart.drawing_text_edit(), Some((id, "€", 1)));
    }

    /// A family tool that owns its text as a multi-line box (the simple annotation). Upstream's
    /// note and comment paint one run, so they edit as a single line.
    fn text_box_chart() -> (ChartEngine, DrawingId) {
        let (mut chart, _) = chart_with(DrawingKind::Text, "");
        let id = chart
            .add_drawing(
                DrawingKind::SimpleAnnotation,
                0,
                vec![DrawingPoint {
                    logical: 8.0,
                    price: 100.0,
                }],
                None,
            )
            .expect("simple annotation");
        chart.build_frame();
        (chart, id)
    }

    #[test]
    fn a_native_typing_session_is_one_undo_step_and_one_sync_revision() {
        let (mut chart, id) = chart_with(DrawingKind::TrendLine, "keep");
        let revision = chart.drawing(id).unwrap().revision;
        let synced = chart.drawing_sync_revision;
        assert!(chart.begin_drawing_text_edit(id, true));
        for input in ["a", "b", "c"] {
            assert!(chart.drawing_text_edit_insert(input));
        }
        assert!(chart.drawing_text_edit_key(DrawingTextEditKey::Backspace, false));
        assert_eq!(text(&chart, id).as_deref(), Some("keepab"));
        assert_eq!(
            chart.drawing_sync_revision, synced,
            "live typing records no sync revision"
        );
        assert!(chart.commit_drawing_text_edit());
        assert_eq!(chart.drawing_sync_revision, synced + 1);
        assert!(chart.undo_drawing());
        assert_eq!(text(&chart, id).as_deref(), Some("keep"));
        assert_eq!(chart.drawing(id).unwrap().revision, revision);
    }

    #[test]
    fn box_labels_keep_line_breaks_and_run_labels_collapse_them() {
        let (mut chart, comment) = text_box_chart();
        assert!(chart.begin_drawing_text_edit(comment, false));
        // Start from an empty box.
        assert!(chart.set_drawing_text_edit("", 0));
        assert!(chart.drawing_text_edit_insert("a\r\nb"));
        assert_eq!(text(&chart, comment).as_deref(), Some("a\nb"));
        assert!(chart.set_drawing_text_edit("a\r\nb\tc", usize::MAX));
        assert_eq!(text(&chart, comment).as_deref(), Some("a\nb c"));
        // The browser mirror can never be refused: a value over the bound is clamped at a
        // character boundary and the caret follows.
        assert!(chart.set_drawing_text_edit(&"€".repeat(40_000), usize::MAX));
        assert_eq!(text(&chart, comment).unwrap().len(), 65_535);
        assert_eq!(chart.drawing_text_edit().unwrap().2, 21_845);
        assert!(chart.cancel_drawing_text_edit());

        let (mut chart, trend) = chart_with(DrawingKind::TrendLine, "");
        assert!(chart.begin_drawing_text_edit(trend, false));
        assert!(chart.set_drawing_text_edit("a\nb", 3));
        assert_eq!(text(&chart, trend).as_deref(), Some("a b"));
    }

    #[test]
    fn a_refused_begin_keeps_the_open_session_and_same_id_begin_is_idempotent() {
        let (mut chart, id) = chart_with(DrawingKind::TrendLine, "keep");
        let other = chart
            .add_drawing(
                DrawingKind::TrendLine,
                0,
                vec![
                    DrawingPoint {
                        logical: 3.0,
                        price: 98.0,
                    },
                    DrawingPoint {
                        logical: 9.0,
                        price: 102.0,
                    },
                ],
                None,
            )
            .expect("second line");
        assert!(chart.set_drawing_locked(other, true));
        assert!(chart.begin_drawing_text_edit(id, false));
        assert!(chart.drawing_text_edit_insert("x"));
        assert!(!chart.begin_drawing_text_edit(other, true));
        assert_eq!(chart.drawing_text_edit(), Some((id, "keepx", 5)));
        assert!(chart.begin_drawing_text_edit(id, true));
        assert_eq!(chart.drawing_text_edit(), Some((id, "keepx", 5)));
        assert!(chart.drawing_text_edit_key(DrawingTextEditKey::Backspace, false));
        assert_eq!(text(&chart, id).as_deref(), Some("keep"));
    }

    #[test]
    fn a_click_places_the_caret_on_the_clicked_line_of_a_text_box() {
        let (mut chart, comment) = text_box_chart();
        chart.set_text_measure(Some(Box::new(|text, _, _, _, _| {
            text.chars().count() as f64 * 10.0
        })));
        chart.build_frame();
        assert!(chart.begin_drawing_text_edit(comment, true));
        assert!(chart.set_drawing_text_edit("ab\ncd", 0));
        let layout = chart.drawing_text_edit_layout(comment).unwrap();
        assert!(layout.multiline);
        assert!(chart.drawing_text_edit_caret_at(layout.x + 10.0, layout.y + layout.line_height));
        assert_eq!(chart.drawing_text_edit(), Some((comment, "ab\ncd", 4)));
        assert!(chart.drawing_text_edit_caret_at(layout.x + 20.0, layout.y));
        assert_eq!(chart.drawing_text_edit(), Some((comment, "ab\ncd", 2)));
    }

    #[test]
    fn an_emptied_text_tool_records_its_update_then_its_delete() {
        let (mut chart, id) = chart_with(DrawingKind::Text, "abc");
        assert!(chart.begin_drawing_text_edit(id, false));
        assert!(chart.set_drawing_text_edit("  ", 2));
        assert!(chart.commit_drawing_text_edit());
        assert!(chart.drawing(id).is_none());
        assert!(chart.undo_drawing(), "the delete");
        assert_eq!(text(&chart, id).as_deref(), Some(""));
        assert!(chart.undo_drawing(), "the update");
        assert_eq!(text(&chart, id).as_deref(), Some("abc"));
    }

    // The emptied-annotation lifecycle genuinely conflicted between the lines: the fork kept an
    // emptied annotation box, upstream removes it. Upstream's rule applies to every text
    // annotation through the session.
    #[test]
    fn an_emptied_text_annotation_records_its_update_then_its_delete() {
        for kind in [DrawingKind::Note, DrawingKind::Callout] {
            let (mut chart, id) = chart_with(kind, "abc");
            assert!(chart.begin_drawing_text_edit(id, false), "{kind:?}");
            assert!(chart.set_drawing_text_edit(" ", 1));
            assert!(chart.commit_drawing_text_edit());
            assert!(
                chart.drawing(id).is_none(),
                "an emptied {kind:?} is removed"
            );
            assert!(chart.undo_drawing(), "the delete");
            assert_eq!(text(&chart, id).as_deref(), Some(""), "{kind:?}");
            assert!(chart.undo_drawing(), "the update");
            assert_eq!(text(&chart, id).as_deref(), Some("abc"), "{kind:?}");
        }
    }

    #[test]
    fn a_fresh_text_annotation_cancelled_empty_is_removed() {
        let (mut chart, id) = chart_with(DrawingKind::Callout, "");
        assert!(chart.begin_drawing_text_edit(id, false));
        assert!(chart.drawing_text_edit_insert("draft"));
        assert!(chart.cancel_drawing_text_edit());
        assert!(chart.drawing(id).is_none());

        // An annotation that began with text keeps it on cancel.
        let (mut chart, note) = chart_with(DrawingKind::Note, "keep");
        assert!(chart.begin_drawing_text_edit(note, false));
        assert!(chart.set_drawing_text_edit("", 0));
        assert!(chart.cancel_drawing_text_edit());
        assert_eq!(text(&chart, note).as_deref(), Some("keep"));
    }

    // The text annotations are the multi-line text owner (R9): their text keeps its lines, and
    // their editor is the multi-line one.
    #[test]
    fn text_annotations_edit_as_multi_line_blocks() {
        for kind in [
            DrawingKind::Note,
            DrawingKind::Comment,
            DrawingKind::Callout,
            DrawingKind::PriceNote,
            DrawingKind::AnchoredText,
        ] {
            let (mut chart, id) = chart_with(kind, "a");
            let layout = chart
                .drawing_text_edit_layout(id)
                .expect("an editor layout");
            assert!(layout.multiline, "{kind:?}");
            assert!(chart.begin_drawing_text_edit(id, false), "{kind:?}");
            assert!(chart.set_drawing_text_edit("a\r\nb", 3));
            assert_eq!(text(&chart, id).as_deref(), Some("a\nb"), "{kind:?}");
        }
    }
}
