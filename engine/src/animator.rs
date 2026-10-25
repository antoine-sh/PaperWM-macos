//! Strip slides: the scroll each workspace shows on screen while it eases
//! toward the workspace's real scroll.

/// A strip sliding from one scroll position to another.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Slide {
    from: f64,
    to: f64,
    shown: f64,
    elapsed: f64,
}

impl Slide {
    fn at(scroll: f64) -> Self {
        Self { from: scroll, to: scroll, shown: scroll, elapsed: 0.0 }
    }

    fn is_moving(&self) -> bool {
        self.shown != self.to
    }
}

/// One slide per workspace.
#[derive(Debug, Clone)]
pub struct Animator {
    slides: Vec<Slide>,
}

impl Animator {
    pub fn new(workspaces: usize) -> Self {
        Self { slides: vec![Slide::default(); workspaces] }
    }

    /// The scroll workspace `index` shows right now.
    pub fn shown(&self, index: usize) -> f64 {
        self.slides[index].shown
    }

    /// Jumps workspace `index` straight to `scroll`.
    pub fn snap(&mut self, index: usize, scroll: f64) {
        self.slides[index] = Slide::at(scroll);
    }

    /// Slides workspace `index` toward `scroll` from wherever it is now,
    /// unless it is already heading there.
    pub fn retarget(&mut self, index: usize, scroll: f64) {
        let slide = &mut self.slides[index];
        if slide.to != scroll {
            *slide = Slide { from: slide.shown, to: scroll, shown: slide.shown, elapsed: 0.0 };
        }
    }

    /// Advances every moving slide by `seconds` of a `duration`-long slide.
    /// Returns whether anything moved.
    pub fn advance(&mut self, seconds: f64, duration: f64) -> bool {
        let mut moved = false;
        for slide in self.slides.iter_mut().filter(|s| s.is_moving()) {
            slide.elapsed += seconds;
            let t = if duration > 0.0 { (slide.elapsed / duration).min(1.0) } else { 1.0 };
            // Ease out: start fast, settle gently.
            let eased = 1.0 - (1.0 - t).powi(3);
            slide.shown = if t >= 1.0 { slide.to } else { slide.from + (slide.to - slide.from) * eased };
            moved = true;
        }
        moved
    }

    pub fn is_moving(&self, index: usize) -> bool {
        self.slides[index].is_moving()
    }

    pub fn is_animating(&self) -> bool {
        self.slides.iter().any(Slide::is_moving)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eases_out_toward_the_target() {
        let mut animator = Animator::new(1);
        animator.snap(0, 0.0);
        animator.retarget(0, 100.0);
        assert!(animator.is_animating());
        assert!(animator.advance(0.5, 1.0));
        assert_eq!(animator.shown(0), 87.5);
        animator.advance(0.5, 1.0);
        assert_eq!(animator.shown(0), 100.0);
        assert!(!animator.is_animating());
        assert!(!animator.advance(0.1, 1.0));
    }

    #[test]
    fn retargeting_to_the_same_scroll_keeps_the_slide_going() {
        let mut animator = Animator::new(1);
        animator.retarget(0, 100.0);
        animator.advance(0.5, 1.0);
        animator.retarget(0, 100.0);
        animator.advance(0.5, 1.0);
        assert_eq!(animator.shown(0), 100.0);
    }
}
