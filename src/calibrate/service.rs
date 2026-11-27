//! Pause/resume the running daemon's capture while the calibration runs.
//!
//! The daemon's manual pause is SIGUSR1 (SPEC §6.2, the Decky plugin's
//! capture switch), so the calibration tool signals the user unit the same
//! way: `systemctl --user kill gamescope-led-sync --signal=SIGUSR1`. SIGUSR1
//! toggles, so the current state is read first from the journal (the daemon
//! logs "capture paused"/"capture resumed", §10.1) and the signal is only
//! sent when the capture is not already paused. The returned guard resumes
//! the capture on drop, restoring the state the calibration found.

use std::{
    path::Path,
    process::{Command, Output},
};

const UNIT: &str = "gamescope-led-sync";

/// Environment for `systemctl --user`/`journalctl --user` when the
/// calibration was launched from Steam (Big Picture), where the usual
/// session variables may be missing: point XDG_RUNTIME_DIR and the bus at
/// the session user's runtime dir (same fallback as the Decky plugin).
fn session_env(cmd: &mut Command) {
    let mut rt = std::env::var_os("XDG_RUNTIME_DIR");
    if rt.is_none()
        && let Some(dir) =
            std::fs::canonicalize(format!("/run/user/{}", unsafe { libc::getuid() })).ok()
        && dir.is_dir()
    {
        rt = Some(dir.into_os_string());
    }
    if let Some(rt) = rt {
        cmd.env("XDG_RUNTIME_DIR", &rt);
        let bus = Path::new(&rt).join("bus");
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() && bus.exists() {
            cmd.env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", bus.display()),
            );
        }
    }
}

fn systemctl(args: &[&str]) -> Option<Output> {
    let mut cmd = Command::new("systemctl");
    cmd.arg("--user").args(args);
    session_env(&mut cmd);
    cmd.output().ok()
}

fn journalctl(args: &[&str]) -> Option<Output> {
    let mut cmd = Command::new("journalctl");
    cmd.arg("--user")
        .arg("-u")
        .arg(UNIT)
        .args(args)
        .arg("--no-pager");
    session_env(&mut cmd);
    cmd.output().ok()
}

/// Whether the user service is currently active.
pub fn service_active() -> bool {
    systemctl(&["show", UNIT, "-p", "ActiveState"])
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l == "ActiveState=active")
        })
        .unwrap_or(false)
}

/// Parse the daemon's journal text: the last pause/resume transition wins.
pub fn parse_paused(journal: &str) -> bool {
    let mut paused = false;
    for line in journal.lines() {
        if line.contains("capture paused") {
            paused = true;
        } else if line.contains("capture resumed") {
            paused = false;
        }
    }
    paused
}

/// Whether the daemon's capture is currently paused, from its journal.
pub fn capture_paused() -> bool {
    journalctl(&["-n", "200"])
        .map(|o| parse_paused(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or(false)
}

/// Send SIGUSR1 to the daemon (toggle pause).
fn signal_sigusr1() -> bool {
    systemctl(&["kill", UNIT, "--signal=SIGUSR1"])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Guard that resumes the capture on drop when it paused it.
pub struct CaptureGuard {
    resume_on_drop: bool,
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        if self.resume_on_drop {
            if signal_sigusr1() {
                tracing::info!("calibration done: capture resumed (SIGUSR1)");
            } else {
                tracing::warn!(
                    "could not resume the capture (SIGUSR1); resume it from the Decky plugin"
                );
            }
        }
    }
}

/// Pause the capture if the service is active and not already paused.
/// Returns a guard that resumes it on drop, or None when nothing was
/// changed (service inactive or already paused).
pub fn pause_capture() -> Option<CaptureGuard> {
    if !service_active() {
        tracing::info!("service {UNIT} is not active; no capture to pause");
        return None;
    }
    if capture_paused() {
        tracing::info!("capture already paused; leaving it as is");
        return None;
    }
    if !signal_sigusr1() {
        tracing::warn!(
            "could not pause the capture (SIGUSR1 failed); the strip may keep moving during calibration"
        );
        return None;
    }
    tracing::info!("capture paused (SIGUSR1) for the calibration");
    Some(CaptureGuard {
        resume_on_drop: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_paused_takes_the_last_transition() {
        let journal = "\
Nov 27 10:00:00 deck gamescope-led-sync[123]: capture paused (SIGUSR1); WLED released
Nov 27 10:05:00 deck gamescope-led-sync[123]: capture resumed (SIGUSR1)
Nov 27 10:06:00 deck gamescope-led-sync[123]: found gamescope node, id 42
";
        assert!(!parse_paused(journal));
        let journal = "\
Nov 27 10:00:00 deck gamescope-led-sync[123]: capture resumed (SIGUSR1)
Nov 27 10:05:00 deck gamescope-led-sync[123]: capture paused (SIGUSR1); WLED released
";
        assert!(parse_paused(journal));
    }

    #[test]
    fn parse_paused_defaults_to_not_paused_on_empty_journal() {
        assert!(!parse_paused(""));
        assert!(!parse_paused(
            "Nov 27 10:00:00 deck gamescope-led-sync[123]: found gamescope node, id 42\n"
        ));
    }
}
