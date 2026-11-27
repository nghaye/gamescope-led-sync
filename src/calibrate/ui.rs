//! The full-screen calibration window: a borderless winit window rendered
//! in software through softbuffer, driven 100% by a gamepad (evdev, see
//! gamepad.rs) with a keyboard fallback (arrows / Enter / Esc / PgUp / PgDn).
//!
//! Each color page shows the target color full-screen (what the TV shows)
//! with a control panel on top: three RGB sliders (what is sent to the
//! strip) and the Précédent / Annuler / Suivant buttons. The final page
//! shows the report with the WLED parameters and a Quitter button.

use {
    crate::calibrate::{
        colors::TestColor,
        fit::{self, Sample},
        gamepad::{Gamepads, Pad},
        render::{Canvas, Fonts, rgb},
        state::{Focus, Outcome, Page, UiState},
    },
    softbuffer::{Context, Surface},
    std::{
        num::NonZeroU32,
        time::{Duration, Instant},
    },
    winit::{
        application::ApplicationHandler,
        event::{ElementState, KeyEvent, WindowEvent},
        event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
        keyboard::{Key, NamedKey},
        raw_window_handle::{DisplayHandle, HasDisplayHandle, HasWindowHandle, WindowHandle},
        window::{Fullscreen, Window, WindowAttributes, WindowId},
    },
};

/// How often the strip color is re-sent while nothing changes (WLED realtime
/// keepalive; the configured timeout_s is >= 1 s, far above this).
const STRIP_TICK: Duration = Duration::from_millis(250);

/// UI tick for the gamepad auto-repeat.
const UI_TICK: Duration = Duration::from_millis(30);

const WHITE: u32 = rgb(245, 245, 250);
const BLACK: u32 = rgb(0, 0, 0);

/// What `run` returns: the final UI state (samples) and the report text when
/// the report page was reached.
pub struct UiResult {
    pub state: UiState,
    pub report: Option<String>,
}

/// Run the calibration UI until the user quits. `on_strip` is called with
/// the current slider color whenever it changes and every STRIP_TICK as a
/// keepalive; the caller scales it to 60% and sends it to WLED.
pub fn run(
    colors: Vec<TestColor>,
    report_path: &str,
    on_strip: &mut dyn FnMut([u8; 3]),
) -> Result<UiResult, String> {
    let pads = Gamepads::open().map_err(|e| format!("cannot read /dev/input: {e}"))?;
    let no_gamepad = pads.count() == 0;
    if no_gamepad {
        tracing::warn!(
            "no gamepad under /dev/input; keyboard fallback: arrows, Enter, Esc, PgUp/PgDn"
        );
    }
    let fonts = Fonts::load().map_err(|e| format!("cannot load the embedded font: {e}"))?;
    let event_loop =
        EventLoop::new().map_err(|e| format!("cannot create the event loop (no display?): {e}"))?;
    let mut app = App {
        window: None,
        surface: None,
        fonts,
        state: UiState::new(colors),
        pads,
        no_gamepad,
        report_path: report_path.to_string(),
        dirty: true,
        last_strip: Instant::now() - STRIP_TICK,
        outcome: Outcome::Continue,
        report: None,
        scroll: 0,
        error: None,
        on_strip,
    };
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("event loop error: {e}"))?;
    if let Some(e) = app.error {
        return Err(e);
    }
    Ok(UiResult {
        state: app.state,
        report: app.report,
    })
}

fn key_to_pad(key: &Key) -> Option<Pad> {
    match key {
        Key::Named(NamedKey::ArrowUp) => Some(Pad::Up),
        Key::Named(NamedKey::ArrowDown) => Some(Pad::Down),
        Key::Named(NamedKey::ArrowLeft) => Some(Pad::Left),
        Key::Named(NamedKey::ArrowRight) => Some(Pad::Right),
        Key::Named(NamedKey::Enter) => Some(Pad::Confirm),
        Key::Named(NamedKey::Escape) => Some(Pad::Cancel),
        Key::Named(NamedKey::PageUp) => Some(Pad::Prev),
        Key::Named(NamedKey::PageDown) => Some(Pad::Next),
        _ => None,
    }
}

struct App<'a> {
    window: Option<Window>,
    surface: Option<Surface<DisplayHandle<'static>, WindowHandle<'static>>>,
    fonts: Fonts,
    state: UiState,
    pads: Gamepads,
    no_gamepad: bool,
    report_path: String,
    dirty: bool,
    last_strip: Instant,
    outcome: Outcome,
    report: Option<String>,
    scroll: usize,
    error: Option<String>,
    on_strip: &'a mut dyn FnMut([u8; 3]),
}

impl<'a> App<'a> {
    fn handle_pad(&mut self, pad: Pad, event_loop: &ActiveEventLoop) {
        // On the report page Up/Down scroll the report instead of moving focus.
        if self.state.page == Page::Report && matches!(pad, Pad::Up | Pad::Down) {
            let max_scroll = self.report_lines().len().saturating_sub(1);
            self.scroll = match pad {
                Pad::Up => self.scroll.saturating_sub(1),
                Pad::Down => (self.scroll + 1).min(max_scroll),
                _ => self.scroll,
            };
            self.dirty = true;
            return;
        }
        let before = self.state.adjusted;
        let outcome = self.state.on_pad(pad, Instant::now());
        if self.state.page == Page::Report && self.report.is_none() && self.state.report_ready {
            self.report = Some(self.compute_report());
        }
        if self.state.adjusted != before {
            (self.on_strip)(self.state.adjusted);
            self.last_strip = Instant::now();
        }
        self.dirty = true;
        if outcome == Outcome::Quit {
            self.outcome = Outcome::Quit;
            event_loop.exit();
        }
    }

    fn report_lines(&self) -> Vec<&str> {
        self.report.as_deref().unwrap_or("").lines().collect()
    }

    fn compute_report(&self) -> String {
        let samples: Vec<Sample> = self.state.samples.iter().flatten().copied().collect();
        let names: Vec<&'static str> = self.state.colors.iter().map(|c| c.name).collect();
        match fit::fit(&samples) {
            Some(f) => fit::report(&samples, &f, &names, &self.report_path),
            None => "Aucun échantillon enregistré.".to_string(),
        }
    }

    fn render(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        let size = window.inner_size();
        let (w, h) = (size.width.max(1) as usize, size.height.max(1) as usize);
        // Draw first (immutable borrow), then hand the pixels to the surface.
        let mut canvas = Canvas::new(w, h);
        self.draw(&mut canvas);
        let Some(surface) = &mut self.surface else {
            return;
        };
        if let (Some(w), Some(h)) = (NonZeroU32::new(w as u32), NonZeroU32::new(h as u32))
            && surface.resize(w, h).is_err()
        {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };
        buffer.copy_from_slice(&canvas.buf);
        let _ = buffer.present();
        self.dirty = false;
    }

    fn draw(&self, c: &mut Canvas) {
        match self.state.page {
            Page::Color(i) => self.draw_color(c, i),
            Page::Report => self.draw_report(c),
        }
    }

    fn draw_color(&self, c: &mut Canvas, i: usize) {
        let (w, h) = (c.w as i32, c.h as i32);
        let target = self.state.colors[i].rgb;
        // Full-screen target color: what the TV shows.
        c.fill(rgb(target[0], target[1], target[2]));

        // Top hint band.
        c.blend_rect(0, 0, w, 36, BLACK, 0.55);
        let hint = if self.no_gamepad {
            "Aucune manette détectée — clavier : flèches = naviguer, gauche/droite = ajuster, Entrée = valider, Échap = annuler, PgUp/PgDn = précédent/suivant"
        } else {
            "Manette : D-pad/stick = naviguer · gauche/droite = ajuster · A = valider · B = annuler · LB/RB = précédent/suivant"
        };
        c.text(&self.fonts.sans, 18.0, 12.0, 25.0, hint, WHITE);

        // Control panel.
        let panel_h = (h * 45 / 100).max(300);
        let py = h - panel_h;
        c.blend_rect(0, py, w, panel_h, rgb(12, 12, 18), 0.85);
        c.rect(0, py, w, 2, rgb(90, 90, 110));

        let title = format!(
            "Calibration LED — couleur {}/{} : {}",
            i + 1,
            self.state.colors.len(),
            self.state.colors[i].name
        );
        c.text(
            &self.fonts.sans_bold,
            30.0,
            40.0,
            py as f32 + 44.0,
            &title,
            WHITE,
        );
        c.text(
            &self.fonts.sans,
            19.0,
            40.0,
            py as f32 + 76.0,
            "L'écran affiche la couleur cible ; réglez R/G/B pour que le ruban LED corresponde.",
            rgb(200, 200, 210),
        );

        // Three sliders.
        let x0 = 90.0f32;
        let bw = (w as f32 * 0.42).max(200.0);
        let bh = 24i32;
        let labels = ["R", "G", "B"];
        let channel_colors = [rgb(255, 60, 60), rgb(60, 255, 60), rgb(80, 80, 255)];
        for ch in 0..3 {
            let y = py as f32 + 116.0 + ch as f32 * 54.0;
            let v = self.state.adjusted[ch];
            let focus = self.state.focus.channel() == Some(ch);
            c.text(
                &self.fonts.sans_bold,
                26.0,
                40.0,
                y + 22.0,
                labels[ch],
                channel_colors[ch],
            );
            c.rect(x0 as i32, y as i32, bw as i32, bh, rgb(42, 42, 52));
            let fw = (bw * v as f32 / 255.0).round() as i32;
            if fw > 0 {
                c.blend_rect(x0 as i32, y as i32, fw, bh, channel_colors[ch], 0.9);
            }
            c.rect(x0 as i32 + fw - 1, y as i32 - 4, 3, bh + 8, WHITE);
            c.text(
                &self.fonts.sans,
                24.0,
                x0 + bw + 18.0,
                y + 22.0,
                &format!("{v:>3}"),
                WHITE,
            );
            if focus {
                c.frame(28, y as i32 - 12, bw as i32 + 250, bh + 24, 2, WHITE);
            }
        }

        // Buttons: Précédent / Annuler / Suivant.
        let labels_btn = [
            ("Précédent", Focus::Prev),
            ("Annuler", Focus::Cancel),
            (
                if i + 1 == self.state.colors.len() {
                    "Terminer"
                } else {
                    "Suivant"
                },
                Focus::Next,
            ),
        ];
        let bw_btn = 210i32;
        let bh_btn = 54i32;
        let gap = 26i32;
        let total = 3 * bw_btn + 2 * gap;
        let mut x = (w - total) / 2;
        let yb = py + panel_h - 76;
        for (label, focus_kind) in labels_btn {
            let focused = self.state.focus == focus_kind;
            let (bg, fg) = if focused {
                (rgb(240, 240, 245), rgb(15, 15, 20))
            } else if focus_kind == Focus::Cancel {
                (rgb(150, 45, 45), WHITE)
            } else {
                (rgb(52, 52, 64), WHITE)
            };
            c.rect(x, yb, bw_btn, bh_btn, bg);
            if focused {
                c.frame(x - 2, yb - 2, bw_btn + 4, bh_btn + 4, 3, WHITE);
            }
            c.text_centered(
                &self.fonts.sans_bold,
                24.0,
                x as f32 + bw_btn as f32 / 2.0,
                yb as f32 + 37.0,
                label,
                fg,
            );
            x += bw_btn + gap;
        }
    }

    fn draw_report(&self, c: &mut Canvas) {
        let (w, h) = (c.w as i32, c.h as i32);
        c.fill(rgb(12, 12, 18));
        c.text(
            &self.fonts.sans_bold,
            32.0,
            40.0,
            52.0,
            "Calibration terminée — rapport",
            WHITE,
        );
        let lines = self.report_lines();
        let line_h = 24.0f32;
        let top = 84.0f32;
        let bottom = h as f32 - 130.0;
        let visible = ((bottom - top) / line_h).floor().max(1.0) as usize;
        let max_scroll = lines.len().saturating_sub(visible);
        let scroll = self.scroll.min(max_scroll);
        let mut y = top + Canvas::ascent(&self.fonts.mono, 18.0);
        for line in lines.iter().skip(scroll).take(visible) {
            c.text(&self.fonts.mono, 18.0, 40.0, y, line, rgb(210, 210, 220));
            y += line_h;
        }
        if max_scroll > 0 {
            c.text(
                &self.fonts.sans,
                16.0,
                40.0,
                h as f32 - 108.0,
                &format!(
                    "haut/bas = faire défiler ({}/{})",
                    scroll + 1,
                    lines.len().max(1)
                ),
                rgb(150, 150, 160),
            );
        }
        // Quit button, always focused.
        let bw_btn = 240i32;
        let bh_btn = 56i32;
        let x = (w - bw_btn) / 2;
        let yb = h - 84;
        c.rect(x, yb, bw_btn, bh_btn, rgb(240, 240, 245));
        c.frame(x - 2, yb - 2, bw_btn + 4, bh_btn + 4, 3, WHITE);
        c.text_centered(
            &self.fonts.sans_bold,
            26.0,
            w as f32 / 2.0,
            yb as f32 + 39.0,
            "Quitter",
            rgb(15, 15, 20),
        );
        c.text(
            &self.fonts.sans,
            17.0,
            40.0,
            h as f32 - 12.0,
            "A / Entrée = quitter (le rapport est sauvegardé) · B / Échap = quitter sans sauvegarder",
            rgb(150, 150, 160),
        );
    }
}

impl<'a> ApplicationHandler for App<'a> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = WindowAttributes::default()
            .with_title("gamescope-led-sync — calibration LED")
            .with_fullscreen(Some(Fullscreen::Borderless(None)));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => w,
            Err(e) => {
                self.error = Some(format!("cannot create the window: {e}"));
                event_loop.exit();
                return;
            },
        };
        // Safety: the raw handles are borrowed from `window`, which is stored
        // in the same struct right after and dropped after the surface (field
        // order), so the handles stay valid for the surface's whole life.
        let context = match window
            .display_handle()
            .map(|d| unsafe { DisplayHandle::borrow_raw(d.as_raw()) })
            .map_err(|e| e.to_string())
            .and_then(|d| Context::new(d).map_err(|e| e.to_string()))
        {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(format!("cannot create the softbuffer context: {e}"));
                event_loop.exit();
                return;
            },
        };
        let surface = match window
            .window_handle()
            .map(|w| unsafe { WindowHandle::borrow_raw(w.as_raw()) })
            .map_err(|e| e.to_string())
            .and_then(|w| Surface::new(&context, w).map_err(|e| e.to_string()))
        {
            Ok(s) => s,
            Err(e) => {
                self.error = Some(format!("cannot create the softbuffer surface: {e}"));
                event_loop.exit();
                return;
            },
        };
        self.window = Some(window);
        self.surface = Some(surface);
        self.dirty = true;
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.outcome = Outcome::Quit;
                event_loop.exit();
            },
            WindowEvent::Resized(size) => {
                self.dirty = true;
                let _ = size;
            },
            WindowEvent::RedrawRequested => self.render(),
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        state: ElementState::Pressed,
                        logical_key,
                        ..
                    },
                ..
            } => {
                if let Some(pad) = key_to_pad(&logical_key) {
                    self.handle_pad(pad, event_loop);
                }
            },
            _ => {},
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let pads = self.pads.poll();
        for pad in pads {
            self.handle_pad(pad, event_loop);
        }
        let repeats = self.state.tick(now);
        for pad in repeats {
            self.handle_pad(pad, event_loop);
        }
        if now.duration_since(self.last_strip) >= STRIP_TICK {
            (self.on_strip)(self.state.adjusted);
            self.last_strip = now;
        }
        if self.dirty
            && let Some(window) = &self.window
        {
            window.request_redraw();
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(now + UI_TICK));
    }
}
