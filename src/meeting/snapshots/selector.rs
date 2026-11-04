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

#[derive(Default)]
pub(super) struct Selector {
    candidate: Option<Candidate>,
    last: Option<Frame>,
}

impl Selector {
    pub fn reset(&mut self) {
        self.candidate = None;
        self.last = None;
    }

    /// Requires two observations at least 500 ms apart. A changing scene is
    /// held in memory until it settles; skipped intermediate frames aren't
    /// claimed as complete visual coverage.
    pub fn observe(&mut self, frame: Frame, now: u64) -> Option<(Frame, u64)> {
        if !frame.valid() {
            self.reset();
            return None;
        }
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
            return None;
        }
        let candidate = self.candidate.as_mut()?;
        candidate.latest = now;
        candidate.observations += 1;
        // Keep the latest frame after the codec has had time to sharpen it.
        candidate.frame = frame;
        if candidate.observations < 2 || now.saturating_sub(candidate.since) < 500 {
            return None;
        }
        let significant = self.last.as_ref().is_none_or(|last| {
            !compatible(last, &candidate.frame) || changed(last, &candidate.frame) >= 3
        });
        if !significant {
            return None;
        }
        let chosen = candidate.frame.clone();
        self.last = Some(chosen.clone());
        Some((chosen, candidate.since))
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
        .filter(|(a, b)| (0..3).any(|channel| a[channel].abs_diff(b[channel]) >= 24))
        .count()
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
    #[test]
    fn keeps_small_persistent_edits_and_does_not_emit_static_frames() {
        let mut selector = Selector::default();
        let mut f = frame();
        assert!(selector.observe(f.clone(), 0).is_none());
        assert!(selector.observe(f.clone(), 500).is_some());
        assert!(selector.observe(f.clone(), 1000).is_none());
        for index in [1, 2, 3] {
            f.pixels[index * 4] = 255;
        }
        assert!(selector.observe(f.clone(), 1500).is_none());
        assert!(selector.observe(f.clone(), 2000).is_some());
        assert!(selector.observe(f, 2500).is_none());
    }
    #[test]
    fn transitions_must_settle_and_loss_of_source_discards_candidates() {
        let mut selector = Selector::default();
        let f = frame();
        selector.observe(f.clone(), 0);
        selector.reset();
        assert!(selector.observe(f.clone(), 1000).is_none());
        let mut other = f;
        other.source = 2;
        assert!(selector.observe(other.clone(), 1500).is_none());
        assert!(selector.observe(other, 2000).is_some());
    }
    #[test]
    fn moving_content_is_not_a_stable_slide() {
        let mut selector = Selector::default();
        for at in 0..5 {
            let mut f = frame();
            f.pixels.fill(at * 40);
            assert!(selector.observe(f, at as u64 * 500).is_none());
        }
    }
}
