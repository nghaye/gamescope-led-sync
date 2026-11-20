//! `--preview-strip`: send the per-segment test pattern to WLED until
//! SIGINT/SIGTERM (ticket 10). The Decky plugin's configuration screen
//! runs this mode with the draft layout as CLI flags, so every change
//! has an immediate effect on the strip without writing the config file.
//!
//! One solid color per segment — bottom red, left blue, top green, right
//! yellow — chained and offset exactly like the live colors, so the
//! preview shows what the counts, start point, direction, and offset do.
//! When the start point is an edge midpoint (ticket 13), that edge is
//! split at the loop's seam: its beginning keeps the edge color, its end
//! is orange, so the two halves of the seam edge read differently and the
//! circular offset is visible on the strip.

use {
    crate::{
        cli::Config,
        geometry::{self, Layout},
        output::WledSender,
    },
    std::{process::ExitCode, thread::sleep, time::Duration},
};

/// How often the pattern is resent; WLED's realtime timeout
/// (`timeout_s`, >= 1 s) is far above this, so the strip stays lit.
const TICK: Duration = Duration::from_millis(250);

/// The test pattern: one solid color per edge, run through the same
/// chain and offset math as the live colors (§5.2/§5.3). On a midpoint
/// start edge, the end of the strip (the loop's seam) is orange.
pub fn segment_pattern(l: &Layout) -> Vec<[u8; 3]> {
    geometry::edge_pattern(
        l,
        &[
            [0, 255, 0],   // top: green
            [255, 255, 0], // right: yellow
            [255, 0, 0],   // bottom: red
            [0, 0, 255],   // left: blue
        ],
        [255, 165, 0], // seam (end of the strip on a midpoint edge): orange
    )
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
        "--preview-strip: packet of {} LEDs, bottom red / left blue / top green / right yellow (seam orange on a midpoint start), sending to {}:{}",
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
    fn pattern_applies_the_offset_as_a_circular_shift() {
        // base chain (clockwise from bl) = [blue, blue, green, green,
        // yellow, yellow, red, red]; offset +2 shifts it two LEDs
        // forward along the loop, so red wraps to the head.
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 2).unwrap();
        let (red, green, blue, yellow) = ([255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]);
        let p = segment_pattern(&l);
        assert_eq!(p.len(), 8, "packet length = count, the offset only shifts");
        assert_eq!(p, vec![red, red, blue, blue, green, green, yellow, yellow]);
    }

    #[test]
    fn pattern_from_a_midpoint_start_splits_the_seam_edge() {
        // bc clockwise: the chain starts in the middle of the bottom
        // edge, so the bottom edge is split at the loop's seam — the
        // beginning of the strip stays red, its end (the last chain
        // positions) is orange.
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bc, Direction::Clockwise, 0).unwrap();
        let (red, green, blue, yellow, orange) = (
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [255, 255, 0],
            [255, 165, 0],
        );
        // base = [L1,L0,T0,T1,R0,R1,B1,B0], s = nl+nt+nr+nb/2 = 7
        // -> chain = B0,L1,L0,T0,T1,R0,R1,B1; head run = [B0] (red),
        // tail run = [B1] (orange).
        assert_eq!(
            segment_pattern(&l),
            vec![red, blue, blue, green, green, yellow, yellow, orange]
        );
        // Counterclockwise from bc starts at the other half of the bottom
        // edge: chain = B1,R1,R0,T1,T0,L0,L1,B0 -> [red, y, y, g, g, b, b, orange].
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bc, Direction::Counterclockwise, 0).unwrap();
        assert_eq!(
            segment_pattern(&l),
            vec![red, yellow, yellow, green, green, blue, blue, orange]
        );
    }

    #[test]
    fn midpoint_split_follows_the_odd_edge_halves_and_the_offset() {
        // bottom = 5, start = bc clockwise: s = nl+nt+nr+nb/2 = 5 in
        // base = [L0,T0,R0,B4,B3,B2,B1,B0], chain = [B2,B1,B0,L0,T0,R0,B4,B3].
        // Head run = 3 red, tail run = 2 orange; offset +1 shifts the
        // whole loop one LED forward, seam colors included.
        let l = Layout::new(1, 1, 5, 1, StartPoint::Bc, Direction::Clockwise, 1).unwrap();
        let (red, green, blue, yellow, orange) = (
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [255, 255, 0],
            [255, 165, 0],
        );
        let chain = [red, red, red, blue, green, yellow, orange, orange];
        let want: Vec<[u8; 3]> = (0..8).map(|i| chain[(i + 8 - 1) % 8]).collect();
        assert_eq!(segment_pattern(&l), want);
    }
}
