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
    pipewire::init();
    capture::install_signal_handler();

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

    // While paused (SIGUSR1, the plugin's Stop capture button), stay in
    // the reconnect wait: on_no_session applies and live reload keeps
    // working, but no capture session runs. The transition is logged
    // once so the plugin can parse it from the journal (§10.1).
    let mut announced_pause = false;
    loop {
        if capture::paused() {
            if !announced_pause {
                tracing::info!("capture paused (SIGUSR1); on_no_session behavior applies");
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

/// Wait before reconnecting. While no session is connected, apply the
/// configured no-session behavior (§6.1/D5): "preset" sends nothing and
/// lets WLED fall back to its own preset after timeout_s, "keep-last"
/// resends the last colors every second, "black" sends a black packet
/// every second; the latter two send nothing until a frame was ever
/// captured. Reload and sender retry run every second here too. On a
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
        if let Some(s) = sender.borrow_mut().as_mut() {
            match c.on_no_session {
                OnNoSession::Preset => {},
                OnNoSession::KeepLast => {
                    if let Some(colors) = last_colors.borrow().as_ref() {
                        s.send(colors);
                    }
                },
                OnNoSession::Black => {
                    if last_colors.borrow().is_some() {
                        s.send(&vec![[0u8; 3]; c.layout.packet_len()]);
                    }
                },
            }
        }
        sleep(step);
        waited += step;
    }
}
