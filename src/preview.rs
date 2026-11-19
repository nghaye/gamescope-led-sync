//! `--preview-strip`: send the per-segment test pattern to WLED until
//! SIGINT/SIGTERM (ticket 10). The Decky plugin's configuration screen
//! runs this mode with the draft layout as CLI flags, so every change
//! has an immediate effect on the strip without writing the config file.
//!
//! One solid color per segment — bottom red, left blue, top green, right
//! yellow — chained and offset exactly like the live colors, so the
//! preview shows what the counts, start point, direction, and offset do.

use {
    crate::{
        cli::Config,
        geometry::{self, EdgeColors, Layout},
        output::WledSender,
    },
    std::{process::ExitCode, thread::sleep, time::Duration},
};

/// How often the pattern is resent; WLED's realtime timeout
/// (`timeout_s`, >= 1 s) is far above this, so the strip stays lit.
const TICK: Duration = Duration::from_millis(250);

/// The test pattern: one solid color per edge, run through the same
/// chain and offset math as the live colors (§5.2/§5.3).
pub fn segment_pattern(l: &Layout) -> Vec<[u8; 3]> {
    let edges = EdgeColors {
        top: vec![[0.0, 255.0, 0.0]; l.top()],       // green
        right: vec![[255.0, 255.0, 0.0]; l.right()], // yellow
        bottom: vec![[255.0, 0.0, 0.0]; l.bottom()], // red
        left: vec![[0.0, 0.0, 255.0]; l.left()],     // blue
    };
    let chained = geometry::chain(&edges, l);
    geometry::apply_offset(&geometry::shape_colors(&chained, 1.0, 1.0, 1.0), l)
}

/// Send the pattern until a signal. The endpoint and layout come from
/// the merged config; the plugin passes them as CLI flags, which
/// override the file (§4.1). Unlike the daemon (§6.0), a sender that
/// cannot be created is fatal here: a preview process has nothing to do
/// without a strip.
pub fn run(cfg: &Config) -> ExitCode {
    let Some(host) = cfg.wled.as_deref() else {
        tracing::error!(
            "--preview-strip needs a WLED host (--wled HOST or [wled] host in the config file)"
        );
        return ExitCode::FAILURE;
    };
    let mut sender = match WledSender::new(host, cfg.port, cfg.timeout_s) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("{e}");
            return ExitCode::FAILURE;
        },
    };
    let colors = segment_pattern(&cfg.layout);
    tracing::info!(
        "--preview-strip: packet of {} LEDs, bottom red / left blue / top green / right yellow, sending to {}:{}",
        cfg.layout.packet_len(),
        host,
        cfg.port,
    );
    loop {
        if crate::capture::interrupted() {
            return ExitCode::SUCCESS;
        }
        sender.send(&colors);
        sleep(TICK);
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::geometry::{Direction, StartPoint},
    };

    #[test]
    fn pattern_colors_each_segment_and_follows_the_chain() {
        // Clockwise from bl, 2 LEDs per edge: base = left bottom-to-top,
        // top left-to-right, right top-to-bottom, bottom right-to-left.
        let (red, green, blue, yellow) = ([255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]);
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        assert_eq!(
            segment_pattern(&l),
            vec![blue, blue, green, green, yellow, yellow, red, red]
        );
        // Counterclockwise from bl walks the base list backwards.
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Counterclockwise, 0).unwrap();
        assert_eq!(
            segment_pattern(&l),
            vec![red, red, yellow, yellow, green, green, blue, blue]
        );
    }

    #[test]
    fn pattern_applies_a_positive_offset_as_a_black_head() {
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 2).unwrap();
        let p = segment_pattern(&l);
        assert_eq!(p.len(), 10, "packet length = count + offset");
        assert_eq!(&p[..2], &[[0, 0, 0], [0, 0, 0]]);
        assert_eq!(&p[2..4], &[[0, 0, 255], [0, 0, 255]], "left = blue");
    }

    #[test]
    fn pattern_from_a_midpoint_start() {
        // bc clockwise: the chain starts in the middle of the bottom edge.
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bc, Direction::Clockwise, 0).unwrap();
        let (red, green, blue, yellow) = ([255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]);
        // base = [L1,L0,T0,T1,R0,R1,B1,B0], s = nl+nt+nr+nb/2 = 7
        // -> chain = B0,L1,L0,T0,T1,R0,R1,B1
        assert_eq!(
            segment_pattern(&l),
            vec![red, blue, blue, green, green, yellow, yellow, red]
        );
    }
}
