//! `--calibrate`: a stand-alone, full-screen, gamepad-driven LED color
//! calibration tool, independent from the daemon and the capture.
//!
//! Flow: pause the running daemon's capture (SIGUSR1, §6.2) so the strip is
//! ours alone, then walk the user through a fixed palette of test colors
//! (colors.rs). On each color the screen shows the target full-screen and
//! the strip shows the slider-adjusted color at 60% brightness; the user
//! tunes R/G/B until screen and strip match, then goes to the next color.
//! On the last color a report page shows the fitted per-channel gains and
//! the WLED white-balance values to enter (Settings > LED preferences >
//! "Color white"), and saves the report to a file. On exit the WLED
//! realtime mode is released (timeout 0 packet) and the capture is resumed
//! if this run paused it.

pub mod colors;
pub mod fit;
pub mod gamepad;
pub mod render;
pub mod service;
pub mod state;
pub mod ui;

use {
    crate::{cli::Config, output::WledSender},
    std::{fs, path::PathBuf, process::ExitCode, time::SystemTime},
};

/// Strip brightness during calibration, as a fraction of the slider value.
pub const CALIBRATION_BRIGHTNESS: f32 = 0.6;

/// Scale a slider color to the calibration brightness.
fn scale_brightness(c: [u8; 3]) -> [u8; 3] {
    c.map(|v| {
        (v as f32 * CALIBRATION_BRIGHTNESS)
            .round()
            .clamp(0.0, 255.0) as u8
    })
}

/// `2026-11-27T10-15-30Z` from a SystemTime (UTC, no dependency).
fn timestamp_utc(now: SystemTime) -> String {
    let secs = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (hh, mm, ss) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 {
        mp + 3
    } else {
        mp - 9
    };
    let y = if mo <= 2 {
        y + 1
    } else {
        y
    };
    format!("{y:04}-{mo:02}-{d:02}T{hh:02}-{mm:02}-{ss:02}Z")
}

/// The report file: `<config dir>/calibration-<UTC timestamp>.txt`.
fn report_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("gamescope-led-sync").join(format!(
        "calibration-{}.txt",
        timestamp_utc(SystemTime::now())
    ))
}

/// The `--calibrate` mode entry point (called before PipeWire is touched).
pub fn run(cfg: &Config) -> ExitCode {
    let Some(host) = cfg.wled.clone() else {
        tracing::error!(
            "--calibrate needs a WLED host (--wled HOST or [wled] host in the config file)"
        );
        return ExitCode::FAILURE;
    };
    // Pause the daemon's capture first; the guard resumes it on drop, after
    // the WLED has been released below.
    let _capture_guard = service::pause_capture();

    let mut sender = match WledSender::new(&host, cfg.port, cfg.timeout_s) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("{e}");
            return ExitCode::FAILURE;
        },
    };
    let packet_len = cfg.layout.packet_len();
    let path = report_path();
    tracing::info!(
        "--calibrate: {} test colors, strip at {:.0}% brightness, {} LEDs per packet, report -> {}",
        colors::test_colors().len(),
        CALIBRATION_BRIGHTNESS * 100.0,
        packet_len,
        path.display()
    );

    let colors = colors::test_colors();
    let dry_run = cfg.dry_run;
    let mut on_strip = |adjusted: [u8; 3]| {
        let scaled = scale_brightness(adjusted);
        if dry_run {
            tracing::info!(
                "--dry-run: would send rgb({},{},{}) to {host}",
                scaled[0],
                scaled[1],
                scaled[2]
            );
        } else {
            sender.send(&vec![scaled; packet_len]);
        }
    };
    let result = ui::run(colors, &path.display().to_string(), &mut on_strip);

    // Release the WLED realtime mode immediately (timeout-0 packet) so the
    // strip goes back to its own preset, then let the guard resume the capture.
    if !dry_run {
        sender.release();
    }

    match result {
        Ok(ui) => {
            let n_samples = ui.state.samples.iter().flatten().count();
            if let Some(report) = &ui.report {
                tracing::info!("calibration finished: {n_samples} colors recorded");
                if let Some(dir) = path.parent() {
                    let _ = fs::create_dir_all(dir);
                }
                match fs::write(&path, report) {
                    Ok(()) => tracing::info!("calibration report saved to {}", path.display()),
                    Err(e) => {
                        tracing::warn!("could not save the report to {}: {e}", path.display())
                    },
                }
            } else {
                tracing::info!("calibration cancelled after {n_samples} colors; no report saved");
            }
            ExitCode::SUCCESS
        },
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brightness_scales_to_60_percent() {
        assert_eq!(scale_brightness([255, 255, 255]), [153, 153, 153]);
        assert_eq!(scale_brightness([100, 0, 200]), [60, 0, 120]);
        assert_eq!(scale_brightness([0, 0, 0]), [0, 0, 0]);
    }

    #[test]
    fn timestamp_formats_the_unix_epoch() {
        assert_eq!(
            timestamp_utc(SystemTime::UNIX_EPOCH),
            "1970-01-01T00-00-00Z"
        );
        // 2026-11-27T10:15:30Z
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_795_774_530);
        assert_eq!(timestamp_utc(t), "2026-11-27T10-15-30Z");
    }
}
