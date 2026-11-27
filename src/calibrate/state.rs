//! The calibration UI state machine, kept pure (no windowing, no I/O) so it
//! is unit-testable: pages, focus, slider values, recorded samples, and the
//! accelerated auto-repeat of held directions.

use {
    crate::calibrate::{colors::TestColor, fit::Sample, gamepad::Pad},
    std::time::Instant,
};

/// A focusable control on a color page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    R,
    G,
    B,
    Prev,
    Cancel,
    Next,
}

impl Focus {
    /// The vertical focus order on a color page.
    const ORDER: [Focus; 6] = [
        Focus::R,
        Focus::G,
        Focus::B,
        Focus::Prev,
        Focus::Cancel,
        Focus::Next,
    ];

    pub fn channel(self) -> Option<usize> {
        match self {
            Focus::R => Some(0),
            Focus::G => Some(1),
            Focus::B => Some(2),
            _ => None,
        }
    }

    fn next(self) -> Focus {
        let i = Self::ORDER.iter().position(|&f| f == self).unwrap();
        Self::ORDER[(i + 1) % Self::ORDER.len()]
    }

    fn prev(self) -> Focus {
        let i = Self::ORDER.iter().position(|&f| f == self).unwrap();
        Self::ORDER[(i + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Color(usize),
    Report,
}

/// What the orchestrator should do after an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    /// Annuler / B: leave now, nothing saved.
    Quit,
}

/// Delay before a held direction starts repeating, and the repeat intervals.
const REPEAT_DELAY_MS: u128 = 400;
const REPEAT_FAST_MS: u128 = 60;
const REPEAT_TURBO_MS: u128 = 25;
const REPEAT_TURBO_AFTER_MS: u128 = 1500;

pub struct UiState {
    pub colors: Vec<TestColor>,
    pub page: Page,
    /// The current slider values (what is sent to the strip).
    pub adjusted: [u8; 3],
    /// One entry per color: the validated (target, adjusted) sample, if the
    /// user already went past this color.
    pub samples: Vec<Option<Sample>>,
    pub focus: Focus,
    /// Held Left/Right for auto-repeat: (direction, first press, last repeat).
    held: Option<(Pad, Instant, Instant)>,
    /// Set once the report page was reached (the report is computed/saved once).
    pub report_ready: bool,
}

impl UiState {
    pub fn new(colors: Vec<TestColor>) -> UiState {
        let adjusted = colors.first().map(|c| c.rgb).unwrap_or([0; 3]);
        UiState {
            samples: vec![None; colors.len()],
            colors,
            page: Page::Color(0),
            adjusted,
            focus: Focus::R,
            held: None,
            report_ready: false,
        }
    }

    /// Enter a color page: sliders start from the recorded sample when the
    /// color was already validated, else from the target itself.
    fn enter_color(&mut self, i: usize) {
        self.page = Page::Color(i);
        self.focus = Focus::R;
        self.adjusted = self.samples[i]
            .map(|s| s.adjusted)
            .unwrap_or(self.colors[i].rgb);
    }

    /// Record the current color and move to the next one, or to the report.
    fn go_next(&mut self) -> Outcome {
        if let Page::Color(i) = self.page {
            self.samples[i] = Some(Sample {
                target: self.colors[i].rgb,
                adjusted: self.adjusted,
            });
            if i + 1 < self.colors.len() {
                self.enter_color(i + 1);
            } else {
                self.page = Page::Report;
                self.report_ready = true;
            }
        }
        Outcome::Continue
    }

    fn go_prev(&mut self) -> Outcome {
        if let Page::Color(i) = self.page
            && i > 0
        {
            self.enter_color(i - 1);
        }
        Outcome::Continue
    }

    /// Apply one Left/Right to the focused slider (clamped 0..=255).
    fn apply_dir(&mut self, pad: Pad) {
        if let (Some(ch), Pad::Left | Pad::Right) = (self.focus.channel(), pad) {
            let delta: i32 = match pad {
                Pad::Right => 1,
                Pad::Left => -1,
                _ => 0,
            };
            let v = self.adjusted[ch] as i32 + delta;
            self.adjusted[ch] = v.clamp(0, 255) as u8;
        }
    }

    /// Handle one pad action. `now` drives the auto-repeat bookkeeping.
    pub fn on_pad(&mut self, pad: Pad, now: Instant) -> Outcome {
        match pad {
            Pad::Up => {
                if self.page != Page::Report {
                    self.focus = self.focus.prev();
                }
                self.held = None;
                Outcome::Continue
            },
            Pad::Down => {
                if self.page != Page::Report {
                    self.focus = self.focus.next();
                }
                self.held = None;
                Outcome::Continue
            },
            Pad::Left | Pad::Right => {
                self.apply_dir(pad);
                self.held = Some((pad, now, now));
                Outcome::Continue
            },
            Pad::Confirm => {
                self.release();
                match self.page {
                    Page::Color(_) => match self.focus {
                        Focus::Prev => self.go_prev(),
                        Focus::Next => self.go_next(),
                        Focus::Cancel => Outcome::Quit,
                        Focus::R | Focus::G | Focus::B => Outcome::Continue,
                    },
                    Page::Report => Outcome::Quit,
                }
            },
            Pad::Cancel => {
                self.release();
                Outcome::Quit
            },
            Pad::Prev => {
                self.release();
                self.go_prev()
            },
            Pad::Next => {
                self.release();
                self.go_next()
            },
        }
    }

    /// Called on every UI tick: emits auto-repeat directions while Left/Right
    /// is held. Returns the generated actions (empty when nothing changed).
    pub fn tick(&mut self, now: Instant) -> Vec<Pad> {
        let Some((pad, first, mut last)) = self.held else {
            return Vec::new();
        };
        let held_ms = now.duration_since(first).as_millis();
        if held_ms < REPEAT_DELAY_MS {
            return Vec::new();
        }
        let interval = if held_ms >= REPEAT_TURBO_AFTER_MS {
            REPEAT_TURBO_MS
        } else {
            REPEAT_FAST_MS
        };
        let mut out = Vec::new();
        while now.duration_since(last).as_millis() >= interval {
            self.apply_dir(pad);
            last += std::time::Duration::from_millis(interval as u64);
            out.push(pad);
        }
        if !out.is_empty() {
            self.held = Some((pad, first, last));
        }
        out
    }

    /// Release the held direction (e.g. when another button is pressed).
    pub fn release(&mut self) {
        self.held = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibrate::colors::test_colors;

    fn state() -> UiState {
        UiState::new(test_colors())
    }

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn starts_on_first_color_with_sliders_at_the_target() {
        let s = state();
        assert_eq!(s.page, Page::Color(0));
        assert_eq!(s.adjusted, s.colors[0].rgb);
        assert_eq!(s.focus, Focus::R);
    }

    #[test]
    fn right_increments_the_focused_channel_and_clamps_at_255() {
        let mut s = state();
        s.colors[0].rgb = [255, 10, 0];
        s.adjusted = [255, 10, 0];
        s.on_pad(Pad::Right, t0());
        assert_eq!(s.adjusted, [255, 10, 0], "clamped at 255");
        s.on_pad(Pad::Left, t0());
        s.on_pad(Pad::Left, t0());
        assert_eq!(s.adjusted, [253, 10, 0]);
        for _ in 0..300 {
            s.on_pad(Pad::Left, t0());
        }
        assert_eq!(s.adjusted[0], 0, "clamped at 0");
    }

    #[test]
    fn focus_cycles_vertically_through_sliders_then_buttons() {
        let mut s = state();
        let seq = [
            Focus::G,
            Focus::B,
            Focus::Prev,
            Focus::Cancel,
            Focus::Next,
            Focus::R,
        ];
        for want in seq {
            s.on_pad(Pad::Down, t0());
            assert_eq!(s.focus, want);
        }
        s.on_pad(Pad::Up, t0());
        assert_eq!(s.focus, Focus::Next);
    }

    #[test]
    fn next_records_the_sample_and_moves_on() {
        let mut s = state();
        s.on_pad(Pad::Right, t0());
        s.on_pad(Pad::Next, t0());
        assert_eq!(s.page, Page::Color(1));
        let mut want = s.colors[0].rgb;
        want[0] = want[0].saturating_add(1);
        assert_eq!(
            s.samples[0],
            Some(Sample {
                target: s.colors[0].rgb,
                adjusted: want
            })
        );
        // New page: sliders reset to the new target.
        assert_eq!(s.adjusted, s.colors[1].rgb);
    }

    #[test]
    fn prev_goes_back_and_restores_the_recorded_adjustment() {
        let mut s = state();
        s.on_pad(Pad::Right, t0());
        s.on_pad(Pad::Next, t0());
        s.on_pad(Pad::Prev, t0());
        assert_eq!(s.page, Page::Color(0));
        let mut want = s.colors[0].rgb;
        want[0] = want[0].saturating_add(1);
        assert_eq!(
            s.adjusted, want,
            "back on a validated color: sliders restore its sample"
        );
    }

    #[test]
    fn prev_on_the_first_color_stays() {
        let mut s = state();
        s.on_pad(Pad::Prev, t0());
        assert_eq!(s.page, Page::Color(0));
    }

    #[test]
    fn cancel_and_b_quit_from_any_page() {
        let mut s = state();
        assert_eq!(s.on_pad(Pad::Cancel, t0()), Outcome::Quit);
        let mut s = state();
        s.focus = Focus::Cancel;
        assert_eq!(s.on_pad(Pad::Confirm, t0()), Outcome::Quit);
        let mut s = state();
        s.on_pad(Pad::Next, t0());
        s.page = Page::Report;
        assert_eq!(s.on_pad(Pad::Confirm, t0()), Outcome::Quit);
    }

    #[test]
    fn last_next_opens_the_report_once() {
        let mut s = state();
        let n = s.colors.len();
        for _ in 0..n {
            s.on_pad(Pad::Next, t0());
        }
        assert_eq!(s.page, Page::Report);
        assert!(s.report_ready);
        assert!(s.samples.iter().all(|x| x.is_some()));
    }

    #[test]
    fn held_direction_repeats_after_the_delay_with_acceleration() {
        let mut s = state();
        s.adjusted = [100, 100, 100]; // leave room above for the repeats
        let t = t0();
        s.on_pad(Pad::Right, t);
        assert_eq!(s.adjusted[0], 101);
        // Before the delay: no repeat.
        assert!(s.tick(t + std::time::Duration::from_millis(200)).is_empty());
        // After the delay: repeats at the fast interval.
        let reps = s.tick(t + std::time::Duration::from_millis(500));
        assert!(!reps.is_empty(), "must repeat after 400 ms");
        assert!(s.adjusted[0] > 101, "repeats applied: {}", s.adjusted[0]);
        // Turbo: a long hold keeps repeating at the faster interval.
        let reps = s.tick(t + std::time::Duration::from_millis(2000));
        assert!(!reps.is_empty());
    }

    #[test]
    fn releasing_stops_the_repeat() {
        let mut s = state();
        let t = t0();
        s.on_pad(Pad::Right, t);
        s.release();
        assert!(
            s.tick(t + std::time::Duration::from_millis(2000))
                .is_empty()
        );
    }
}
