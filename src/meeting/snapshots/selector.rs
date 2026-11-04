//! A small, conservative selector. Small persistent edits are kept; similarity
//! never merges two different images elsewhere in the meeting.

#[derive(Clone)]
pub(super) struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub source: u64,
    pub window: u32,
    pub binding: u64,
}

impl Frame {
    pub fn valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.source != 0
            && self.window != 0
            && self.binding != 0
            && self.width <= 1600
            && self.height <= 1600
            && self.pixels.len() == self.width as usize * self.height as usize * 4
    }
}

struct Candidate {
    frame: Frame,
    since: u64,
    latest: u64,
    observations: u32,
}

pub(super) struct Selection {
    pub frame: Frame,
    pub since: u64,
    pub sparse: bool,
}

#[derive(Default)]
pub(super) struct Selector {
    candidate: Option<Candidate>,
    recent: std::collections::VecDeque<Frame>,
    observing_since: Option<u64>,
    saved_at: Option<u64>,
}

impl Selector {
    /// Interruptions invalidate candidates, not the comparison history. This
    /// also prevents deleting an image from making the same scene new again.
    pub fn reset(&mut self) {
        self.candidate = None;
        self.observing_since = None;
    }

    /// Commit comparison history only after storage accepted the selection.
    /// A failed write must remain retryable.
    pub fn remember(&mut self, frame: Frame, now: u64) {
        self.recent.push_front(frame);
        self.recent.truncate(2);
        self.saved_at = Some(now);
    }

    /// Large settled changes need 500 ms; small edits need 1500 ms so a
    /// blinking caret cannot exhaust the image limit. Continuous motion gets
    /// an explicitly sparse frame after 2 s initially, then every 30 s.
    /// These are conservative starting values, not measured recall guarantees.
    pub fn observe(&mut self, frame: Frame, now: u64) -> Option<Selection> {
        if !frame.valid() {
            self.reset();
            return None;
        }
        let started = *self.observing_since.get_or_insert(now);
        let same_scene = self.candidate.as_ref().is_some_and(|c| {
            compatible(&c.frame, &frame)
                && changed(&c.frame, &frame) < 3
                && now.saturating_sub(c.latest) <= 3000
        });
        if !same_scene {
            self.candidate = Some(Candidate {
                frame,
                since: now,
                latest: now,
                observations: 1,
            });
        } else {
            let candidate = self.candidate.as_mut()?;
            candidate.latest = now;
            candidate.observations += 1;
            candidate.frame = frame;
        }
        let candidate = self.candidate.as_ref()?;
        if self
            .recent
            .iter()
            .any(|last| compatible(last, &candidate.frame) && changed(last, &candidate.frame) < 3)
        {
            return None;
        }
        let small_edit = self.recent.front().is_some_and(|last| {
            compatible(last, &candidate.frame)
                && changed(last, &candidate.frame) < (candidate.frame.pixels.len() / 800).max(4)
        });
        let stable = candidate.observations >= 2
            && now.saturating_sub(candidate.since) >= if small_edit { 1500 } else { 500 };
        let sparse = !stable
            && match self.saved_at {
                Some(at) => now.saturating_sub(at.max(started)) >= 30_000,
                None => now.saturating_sub(started) >= 2000,
            };
        (stable || sparse).then(|| Selection {
            frame: candidate.frame.clone(),
            since: candidate.since,
            sparse,
        })
    }
}

fn compatible(a: &Frame, b: &Frame) -> bool {
    a.width == b.width
        && a.height == b.height
        && a.source == b.source
        && a.window == b.window
        && a.binding == b.binding
}

fn changed(a: &Frame, b: &Frame) -> usize {
    a.pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.pixels.as_chunks::<4>().0.iter())
        .filter(|(a, b)| material_pixel_change(a, b))
        .count()
}

/// Video decoding and resampling can shift chroma at glyph edges without a
/// meaningful brightness change. Compare both brightness and strong color
/// changes, rather than letting one noisy RGB channel create a new slide.
fn material_pixel_change(a: &[u8; 4], b: &[u8; 4]) -> bool {
    let brightness =
        |p: &[u8; 4]| 54 * i32::from(p[0]) + 183 * i32::from(p[1]) + 19 * i32::from(p[2]);
    (brightness(a) - brightness(b)).abs() >= 16 * 256
        || (0..3).any(|channel| a[channel].abs_diff(b[channel]) >= 48)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame() -> Frame {
        Frame {
            width: 100,
            height: 100,
            pixels: vec![0; 40000],
            source: 1,
            window: 1,
            binding: 1,
        }
    }
    fn accept(selector: &mut Selector, frame: Frame, at: u64) -> bool {
        if let Some(chosen) = selector.observe(frame, at) {
            selector.remember(chosen.frame, at);
            true
        } else {
            false
        }
    }
    #[test]
    fn keeps_small_persistent_edits_and_survives_capture_resets() {
        let mut selector = Selector::default();
        let mut f = frame();
        assert!(!accept(&mut selector, f.clone(), 0));
        assert!(accept(&mut selector, f.clone(), 500));
        selector.reset();
        assert!(!accept(&mut selector, f.clone(), 1000));
        assert!(!accept(&mut selector, f.clone(), 1500));
        for index in [1, 2, 3] {
            f.pixels[index * 4] = 255;
        }
        assert!(!accept(&mut selector, f.clone(), 2000));
        assert!(!accept(&mut selector, f.clone(), 2500));
        assert!(accept(&mut selector, f.clone(), 3500));
        assert!(!accept(&mut selector, f, 4000));
    }
    #[test]
    fn deletion_reset_keeps_comparison_despite_codec_noise() {
        let mut selector = Selector::default();
        accept(&mut selector, frame(), 0);
        accept(&mut selector, frame(), 500);
        selector.reset();
        let mut noisy = frame();
        noisy.pixels.fill(12);
        assert!(!accept(&mut selector, noisy.clone(), 1000));
        assert!(!accept(&mut selector, noisy, 2500));
    }
    #[test]
    fn text_edge_chroma_noise_does_not_create_a_new_snapshot() {
        let mut selector = Selector::default();
        let mut original = frame();
        for y in 20..40 {
            for x in 10..80 {
                if (x + y) % 3 == 0 {
                    let p = (y * 100 + x) * 4;
                    original.pixels[p..p + 4].copy_from_slice(&[20, 80, 110, 255]);
                }
            }
        }
        accept(&mut selector, original.clone(), 0);
        assert!(accept(&mut selector, original.clone(), 500));
        let mut noisy = original;
        for p in noisy.pixels.chunks_exact_mut(4) {
            if p[0] == 20 {
                p.copy_from_slice(&[50, 74, 112, 255]);
            }
        }
        selector.reset();
        assert!(!accept(&mut selector, noisy.clone(), 1000));
        assert!(!accept(&mut selector, noisy, 3000));
    }

    #[test]
    fn low_contrast_digits_and_strong_equal_brightness_colors_remain_material() {
        let mut selector = Selector::default();
        let mut original = frame();
        let two = [
            0b11110, 0b00001, 0b00001, 0b01110, 0b10000, 0b10000, 0b11111,
        ];
        let three = [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ];
        let paint = |frame: &mut Frame, rows: [u8; 7]| {
            for (y, row) in rows.iter().enumerate() {
                for x in 0..5 {
                    let grey = if row & (1 << x) == 0 { 0 } else { 20 };
                    let p = ((20 + y) * 100 + 20 + x) * 4;
                    frame.pixels[p..p + 4].copy_from_slice(&[grey, grey, grey, 255]);
                }
            }
        };
        paint(&mut original, two);
        accept(&mut selector, original.clone(), 0);
        assert!(accept(&mut selector, original.clone(), 500));
        paint(&mut original, three);
        assert!(!accept(&mut selector, original.clone(), 1000));
        assert!(accept(&mut selector, original, 2500));
        assert!(material_pixel_change(&[0, 90, 0, 255], &[255, 14, 0, 255]));
    }

    #[test]
    fn failed_storage_does_not_suppress_a_retry() {
        let mut selector = Selector::default();
        selector.observe(frame(), 0);
        assert!(selector.observe(frame(), 500).is_some());
        selector.reset();
        selector.observe(frame(), 1000);
        assert!(selector.observe(frame(), 1500).is_some());
    }
    #[test]
    fn source_changes_need_fresh_observations() {
        let mut selector = Selector::default();
        accept(&mut selector, frame(), 0);
        accept(&mut selector, frame(), 500);
        let mut other = frame();
        other.source = 2;
        assert!(!accept(&mut selector, other.clone(), 1000));
        assert!(accept(&mut selector, other, 1500));
    }
    #[test]
    fn blinking_caret_does_not_fill_the_meeting() {
        let mut selector = Selector::default();
        let base = frame();
        accept(&mut selector, base.clone(), 0);
        accept(&mut selector, base.clone(), 500);
        let mut saved = 0;
        for step in 2..1200 {
            let mut f = base.clone();
            if step % 2 == 0 {
                for pixel in 0..4 {
                    f.pixels[pixel * 4] = 255;
                }
            }
            saved += usize::from(accept(&mut selector, f, step * 500));
        }
        assert!(saved <= 1, "saved {saved} caret frames");
    }
    #[test]
    fn continuous_motion_produces_only_sparse_representatives() {
        let mut selector = Selector::default();
        let mut saved = Vec::new();
        for step in 0..130 {
            let mut f = frame();
            f.pixels.fill(((step % 5) * 40) as u8);
            if let Some(chosen) = selector.observe(f, step * 500) {
                assert!(chosen.sparse);
                saved.push(step * 500);
                selector.remember(chosen.frame, step * 500);
            }
        }
        assert_eq!(saved.len(), 3);
        assert_eq!(saved[0], 2000);
        assert!(saved.windows(2).all(|pair| pair[1] - pair[0] >= 30_000));
    }
}
