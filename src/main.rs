//! Ambient LED sync for gamescope sessions.
//!
//! Reads the gamescope compositor's PipeWire video stream on the same
//! machine, computes one color per LED around the screen edges, and
//! streams them to a WLED controller as DRGB realtime packets over
//! UDP. The HDMI signal is never touched.
//!
//! This client speaks gamescope's custom `requested_size`
//! negotiation, so gamescope downscales the capture on the GPU and
//! ships small frames.

mod capture;
mod cli;
mod config;
mod geometry;
mod output;
mod preview;
mod reload;
mod setup;

use {
    crate::config::{Config, OnNoSession},
    std::{
        cell::{Cell, RefCell},
        process::ExitCode,
        rc::Rc,
        thread::sleep,
        time::{Duration, Instant},
    },
};

const RECONNECT_DELAY: Duration = Duration::from_secs(2);

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let parsed = cli::parse();
    capture::install_signal_handler();

    // --preview-strip is a mode, not a flag: the Decky plugin's
    // configuration screen runs it with the draft layout as CLI flags
    // so changes light the strip immediately without touching the file.
    if parsed.config.preview_strip {
        return preview::run(&parsed.config);
    }

    pipewire::init();

    // [capture] start_paused (D12): start paused when the config says
    // so; SIGUSR1 still toggles on top, and a reload applies a change of
    // the key immediately (§7, reload.rs).
    capture::set_paused(parsed.config.start_paused);
    if parsed.config.start_paused {
        tracing::info!("capture paused at startup ([capture] start_paused = true)");
    }

    // The config, the WLED sender, and the last colors sent are shared
    // across sessions and mutated by the reload path, so a reconnect
    // and a live reload both act on the current values (§7).
    let cfg = Rc::new(RefCell::new(parsed.config));
    let reloader = Rc::new(RefCell::new(reload::Reloader::new(parsed.path)));
    let layout_line = cfg.borrow().layout.summary();
    tracing::info!("layout: {layout_line}");
    let sender: Rc<RefCell<Option<output::WledSender>>> = Rc::new(RefCell::new(None));
    let last_sender_try = Rc::new(Cell::new(Instant::now() - reload::SENDER_RETRY));
    let last_colors: Rc<RefCell<Option<Vec<[u8; 3]>>>> = Rc::new(RefCell::new(None));

    // A sender that cannot be created (unresolvable host, bind error)
    // never exits the daemon: it is retried every 5 s (§6.0).
    reload::ensure_sender(&cfg.borrow(), &sender, &last_sender_try);
    {
        let c = cfg.borrow();
        if sender.borrow().is_none() {
            if c.dry_run {
                tracing::info!("--dry-run: packets are computed and logged, never sent");
            } else if c.wled.is_none() {
                tracing::info!("no --wled given: dry run, nothing is sent");
            }
        }
    }

    // While paused (SIGUSR1, the plugin's Capture switch), stay in the
    // reconnect wait: live reload keeps working, no capture session
    // runs, and nothing is sent to the WLED so its realtime timeout
    // releases the strip (ticket 11). The transition is logged once so
    // the plugin can parse it from the journal (§10.1).
    let mut announced_pause = false;
    loop {
        if capture::paused() {
            if !announced_pause {
                tracing::info!("capture paused (SIGUSR1); WLED released, no packets while paused");
                announced_pause = true;
            }
            wait_and_keepalive(&cfg, &sender, &last_colors, &reloader, &last_sender_try);
            if capture::interrupted() {
                return ExitCode::SUCCESS;
            }
            continue;
        }
        if announced_pause {
            tracing::info!("capture resumed (SIGUSR1)");
            announced_pause = false;
        }
        match capture::run(&cfg, &sender, &last_colors, &reloader, &last_sender_try) {
            Ok(capture::Reason::OnceDone) => return ExitCode::SUCCESS,
            Ok(capture::Reason::Interrupted) => {
                tracing::info!("interrupted; disconnected cleanly");
                return ExitCode::SUCCESS;
            },
            Ok(capture::Reason::OnceFailed(msg)) => {
                tracing::warn!("{msg}");
                return ExitCode::FAILURE;
            },
            Ok(capture::Reason::Stopped(msg)) => {
                if cfg.borrow().once {
                    tracing::warn!("{msg}");
                    return ExitCode::FAILURE;
                }
                tracing::warn!("session ended ({msg}); reconnecting in 2 s");
            },
            Ok(capture::Reason::Paused) => continue,
            Err(msg) => {
                if cfg.borrow().once {
                    tracing::warn!("{msg}");
                    return ExitCode::FAILURE;
                }
                tracing::warn!("{msg}; retrying in 2 s");
            },
        }
        // A signal during the session or the wait must end the process,
        // not trigger another reconnect.
        if capture::interrupted() {
            return ExitCode::SUCCESS;
        }
        wait_and_keepalive(&cfg, &sender, &last_colors, &reloader, &last_sender_try);
        if capture::interrupted() {
            return ExitCode::SUCCESS;
        }
    }
}

/// The no-session packet to send this wait tick, if any (§6.1/D5).
/// While the capture is manually paused (§6.2), nothing is sent
/// whatever `on_no_session` says: the WLED is released so its realtime
/// timeout can hand the strip back to its own preset (ticket 11).
fn no_session_packet(
    c: &Config,
    paused: bool,
    last_colors: &Option<Vec<[u8; 3]>>,
) -> Option<Vec<[u8; 3]>> {
    if paused {
        return None;
    }
    match c.on_no_session {
        OnNoSession::Preset => None,
        OnNoSession::KeepLast => last_colors.clone(),
        OnNoSession::Black => last_colors
            .as_ref()
            .map(|_| vec![[0u8; 3]; c.layout.packet_len()]),
    }
}

/// Wait before reconnecting. While no session is connected, apply the
/// configured no-session behavior (§6.1/D5) via [`no_session_packet`] —
/// unless the capture is manually paused, in which case nothing is sent
/// at all. Reload and sender retry run every second here too. On a
/// signal the loop returns without sending anything (§6.1: clean
/// shutdown sends nothing).
fn wait_and_keepalive(
    cfg: &Rc<RefCell<Config>>,
    sender: &Rc<RefCell<Option<output::WledSender>>>,
    last_colors: &Rc<RefCell<Option<Vec<[u8; 3]>>>>,
    reloader: &Rc<RefCell<reload::Reloader>>,
    last_sender_try: &Rc<Cell<Instant>>,
) {
    let step = Duration::from_secs(1);
    let mut waited = Duration::ZERO;
    while waited < RECONNECT_DELAY {
        if capture::interrupted() {
            return;
        }
        reload::reload_if_changed(cfg, sender, reloader);
        let c = cfg.borrow().clone();
        reload::ensure_sender(&c, sender, last_sender_try);
        if let Some(s) = sender.borrow_mut().as_mut()
            && let Some(colors) = no_session_packet(&c, capture::paused(), &last_colors.borrow())
        {
            s.send(&colors);
        }
        sleep(step);
        waited += step;
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::geometry::{Direction, Layout, StartPoint},
    };

    // Ticket 11: a manual pause releases the WLED — no packet may go out
    // while paused, whatever on_no_session says, so WLED's realtime
    // timeout hands the strip back.
    #[test]
    fn paused_releases_the_wled_regardless_of_on_no_session() {
        let mut c = Config::defaults();
        c.layout = Layout::new(10, 10, 10, 10, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        let last = Some(vec![[9u8; 3]; c.layout.packet_len()]);
        for behavior in [
            OnNoSession::Preset,
            OnNoSession::KeepLast,
            OnNoSession::Black,
        ] {
            c.on_no_session = behavior;
            assert!(
                no_session_packet(&c, true, &last).is_none(),
                "paused must send nothing with {behavior:?}"
            );
        }
        // Unpaused, §6.1 still applies as before.
        c.on_no_session = OnNoSession::KeepLast;
        assert_eq!(no_session_packet(&c, false, &last).as_ref(), last.as_ref());
        c.on_no_session = OnNoSession::Black;
        let black = no_session_packet(&c, false, &last).unwrap();
        assert_eq!(black.len(), c.layout.packet_len());
        assert!(black.iter().all(|px| *px == [0u8; 3]));
        c.on_no_session = OnNoSession::Preset;
        assert!(no_session_packet(&c, false, &last).is_none());
        // No frame ever captured: keep-last/black stay silent (§6.1).
        c.on_no_session = OnNoSession::KeepLast;
        assert!(no_session_packet(&c, false, &None).is_none());
        c.on_no_session = OnNoSession::Black;
        assert!(no_session_packet(&c, false, &None).is_none());
    }
}
