//! Pure geometry and color math: LED layout, edge zone means, chain
//! ordering, and color shaping. No I/O, mirrors the Python
//! implementation exactly.

use std::ops::Range;

/// Where the strip's first LED sits: a corner or an edge midpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartPoint {
    Bl,
    Bc,
    Br,
    Rc,
    Tr,
    Tc,
    Tl,
    Lc,
}

impl StartPoint {
    pub const CODES: [&'static str; 8] = ["bl", "bc", "br", "rc", "tr", "tc", "tl", "lc"];

    pub fn parse(s: &str) -> Option<StartPoint> {
        match s {
            "bl" => Some(StartPoint::Bl),
            "bc" => Some(StartPoint::Bc),
            "br" => Some(StartPoint::Br),
            "rc" => Some(StartPoint::Rc),
            "tr" => Some(StartPoint::Tr),
            "tc" => Some(StartPoint::Tc),
            "tl" => Some(StartPoint::Tl),
            "lc" => Some(StartPoint::Lc),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            StartPoint::Bl => "bl",
            StartPoint::Bc => "bc",
            StartPoint::Br => "br",
            StartPoint::Rc => "rc",
            StartPoint::Tr => "tr",
            StartPoint::Tc => "tc",
            StartPoint::Tl => "tl",
            StartPoint::Lc => "lc",
        }
    }
}

/// Which way the strip runs from its start point, seen from the front.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Clockwise,
    Counterclockwise,
}

impl Direction {
    pub const CODES: [&'static str; 2] = ["clockwise", "counterclockwise"];

    pub fn parse(s: &str) -> Option<Direction> {
        match s {
            "clockwise" => Some(Direction::Clockwise),
            "counterclockwise" => Some(Direction::Counterclockwise),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Direction::Clockwise => "clockwise",
            Direction::Counterclockwise => "counterclockwise",
        }
    }
}

/// Why a Layout was rejected.
#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("layout has zero LEDs")]
    ZeroLeds,
    #[error("{count} LEDs exceed one DRGB packet (490)")]
    TooManyLeds { count: usize },
    #[error("packet length {len} exceeds one DRGB packet (490)")]
    PacketTooLong { len: usize },
    #[error("start {start} is a midpoint and needs its edge to have at least 1 LED")]
    MidpointOnEmptyEdge { start: &'static str },
}

/// LED counts per edge plus the strip's physical orientation. A Layout
/// only exists if its packet (chain plus a positive offset's leading
/// black LEDs) fits one DRGB packet (490 LEDs).
#[derive(Clone, Debug)]
pub struct Layout {
    top: usize,
    right: usize,
    bottom: usize,
    left: usize,
    start: StartPoint,
    direction: Direction,
    offset: i32,
}

impl Layout {
    pub fn new(
        top: usize,
        right: usize,
        bottom: usize,
        left: usize,
        start: StartPoint,
        direction: Direction,
        offset: i32,
    ) -> Result<Layout, LayoutError> {
        let count = top + right + bottom + left;
        if count == 0 {
            return Err(LayoutError::ZeroLeds);
        }
        if count > 490 {
            return Err(LayoutError::TooManyLeds { count });
        }
        let len = count + offset.max(0) as usize;
        if len > 490 {
            return Err(LayoutError::PacketTooLong { len });
        }
        // A midpoint start on an edge with no LEDs is meaningless; a
        // corner start degenerates to the next edge in rotation order.
        let midpoint_edge = match start {
            StartPoint::Lc => Some(left),
            StartPoint::Tc => Some(top),
            StartPoint::Rc => Some(right),
            StartPoint::Bc => Some(bottom),
            _ => None,
        };
        if let Some(n) = midpoint_edge
            && n == 0
        {
            return Err(LayoutError::MidpointOnEmptyEdge {
                start: start.name(),
            });
        }
        Ok(Layout {
            top,
            right,
            bottom,
            left,
            start,
            direction,
            offset,
        })
    }

    pub fn top(&self) -> usize {
        self.top
    }

    pub fn right(&self) -> usize {
        self.right
    }

    pub fn bottom(&self) -> usize {
        self.bottom
    }

    pub fn left(&self) -> usize {
        self.left
    }

    pub fn start(&self) -> StartPoint {
        self.start
    }

    pub fn direction(&self) -> Direction {
        self.direction
    }

    pub fn offset(&self) -> i32 {
        self.offset
    }

    pub fn count(&self) -> usize {
        self.top + self.right + self.bottom + self.left
    }

    /// LEDs in the DRGB packet: the chain plus a positive offset's
    /// leading black LEDs.
    pub fn packet_len(&self) -> usize {
        self.count() + self.offset.max(0) as usize
    }
}

/// A borrowed BGRx frame plus the subsampling step. With gamescope's
/// requested_size in effect the frame is already small and step is 1;
/// the step covers the full-resolution fallback.
pub struct Frame<'a> {
    pub data: &'a [u8],
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub step: usize,
}

pub struct EdgeColors {
    pub top: Vec<[f32; 3]>,
    pub right: Vec<[f32; 3]>,
    pub bottom: Vec<[f32; 3]>,
    pub left: Vec<[f32; 3]>,
}

fn sampled_count(len: usize, step: usize) -> usize {
    len.div_ceil(step)
}

fn pixel(f: &Frame, sx: usize, sy: usize) -> [f32; 3] {
    // BGRx byte order in memory; returned as RGB.
    let i = sy * f.step * f.stride + sx * f.step * 4;
    [f.data[i + 2] as f32, f.data[i + 1] as f32, f.data[i] as f32]
}

fn zones_x(f: &Frame, rows: Range<usize>, n: usize, sw: usize) -> Vec<[f32; 3]> {
    let mut out = Vec::with_capacity(n);
    for z in 0..n {
        let x0 = z * sw / n;
        let x1 = ((z + 1) * sw / n).max(x0 + 1);
        let (mut acc, mut cnt) = ([0f32; 3], 0f32);
        for sy in rows.clone() {
            for sx in x0..x1 {
                let p = pixel(f, sx, sy);
                acc[0] += p[0];
                acc[1] += p[1];
                acc[2] += p[2];
                cnt += 1.0;
            }
        }
        out.push([acc[0] / cnt, acc[1] / cnt, acc[2] / cnt]);
    }
    out
}

fn zones_y(f: &Frame, cols: Range<usize>, n: usize, sh: usize) -> Vec<[f32; 3]> {
    let mut out = Vec::with_capacity(n);
    for z in 0..n {
        let y0 = z * sh / n;
        let y1 = ((z + 1) * sh / n).max(y0 + 1);
        let (mut acc, mut cnt) = ([0f32; 3], 0f32);
        for sy in y0..y1 {
            for sx in cols.clone() {
                let p = pixel(f, sx, sy);
                acc[0] += p[0];
                acc[1] += p[1];
                acc[2] += p[2];
                cnt += 1.0;
            }
        }
        out.push([acc[0] / cnt, acc[1] / cnt, acc[2] / cnt]);
    }
    out
}

/// Per-edge zone colors in canonical reading order: top and bottom run
/// left to right, left and right run top to bottom.
pub fn edge_means(f: &Frame, band_frac: f64, l: &Layout) -> EdgeColors {
    let sw = sampled_count(f.width, f.step);
    let sh = sampled_count(f.height, f.step);
    let band = ((band_frac * sh as f64).round() as usize).clamp(1, sh);
    let band_w = band.min(sw);
    EdgeColors {
        top: zones_x(f, 0..band, l.top, sw),
        bottom: zones_x(f, sh - band..sh, l.bottom, sw),
        left: zones_y(f, 0..band_w, l.left, sh),
        right: zones_y(f, sw - band_w..sw, l.right, sh),
    }
}

/// Index in the base list where the chain starts, per the locked
/// convention: clockwise `chain[i] = base[(s+i) mod count]`,
/// counterclockwise `chain[i] = base[(s-i) mod count]`. A corner start
/// with an empty adjacent edge wraps around (rem_euclid), which
/// degenerates to the next edge in rotation order.
fn start_index(l: &Layout) -> usize {
    let nt = l.top() as isize;
    let nr = l.right() as isize;
    let nb = l.bottom() as isize;
    let nl = l.left() as isize;
    let count = l.count() as isize;
    let s = match (l.start(), l.direction()) {
        (StartPoint::Bl, Direction::Clockwise) => 0,
        (StartPoint::Bl, Direction::Counterclockwise) => count - 1,
        (StartPoint::Tl, Direction::Clockwise) => nl,
        (StartPoint::Tl, Direction::Counterclockwise) => nl - 1,
        (StartPoint::Tr, Direction::Clockwise) => nl + nt,
        (StartPoint::Tr, Direction::Counterclockwise) => nl + nt - 1,
        (StartPoint::Br, Direction::Clockwise) => nl + nt + nr,
        (StartPoint::Br, Direction::Counterclockwise) => nl + nt + nr - 1,
        (StartPoint::Lc, Direction::Clockwise) => nl / 2,
        (StartPoint::Lc, Direction::Counterclockwise) => nl - 1 - nl / 2,
        (StartPoint::Tc, Direction::Clockwise) => nl + nt / 2,
        (StartPoint::Tc, Direction::Counterclockwise) => nl + nt - 1 - nt / 2,
        (StartPoint::Rc, Direction::Clockwise) => nl + nt + nr / 2,
        (StartPoint::Rc, Direction::Counterclockwise) => nl + nt + nr - 1 - nr / 2,
        (StartPoint::Bc, Direction::Clockwise) => nl + nt + nr + nb / 2,
        (StartPoint::Bc, Direction::Counterclockwise) => count - 1 - nb / 2,
    };
    s.rem_euclid(count) as usize
}

/// Order edge zones along the physical strip, first LED first.
///
/// The base cyclic list is clockwise from the bottom-left corner, seen
/// from the front: left edge bottom-to-top, top left-to-right, right
/// edge top-to-bottom, bottom right-to-left. chain[0] is the zone
/// physically at the start point; chain[i] is the i-th zone along the
/// strip in the chosen direction.
pub fn chain(e: &EdgeColors, l: &Layout) -> Vec<[f32; 3]> {
    let nl = l.left();
    let nb = l.bottom();
    let count = l.count();
    let mut base = Vec::with_capacity(count);
    for k in 0..nl {
        base.push(e.left[nl - 1 - k]);
    }
    base.extend(e.top.iter().copied());
    base.extend(e.right.iter().copied());
    for k in 0..nb {
        base.push(e.bottom[nb - 1 - k]);
    }
    let s = start_index(l);
    (0..count)
        .map(|i| match l.direction() {
            Direction::Clockwise => base[(s + i) % count],
            Direction::Counterclockwise => base[(s + count - i) % count],
        })
        .collect()
}

/// Build the DRGB packet from the chain: a positive offset pads the
/// head with black LEDs (packet length = count + offset), a negative
/// offset rotates the chain. Works on zones ([f32; 3]) or on graded
/// colors ([u8; 3]); black is the color type's default.
pub fn apply_offset<T: Copy + Default>(chained: &[T], l: &Layout) -> Vec<T> {
    let count = l.count();
    let offset = l.offset() as isize;
    debug_assert_eq!(chained.len(), count);
    (0..l.packet_len())
        .map(|i| {
            let i = i as isize;
            if offset > 0 && i < offset {
                T::default()
            } else {
                chained[(i - offset).rem_euclid(count as isize) as usize]
            }
        })
        .collect()
}

/// The color grading, on one pixel: saturation gain then gamma, both
/// no-ops at 1.0. The saturation compensates the muted look of
/// gamescope's HDR-to-SDR capture; the gamma lifts its crushed
/// shadows. Kept in one place so the LED output and every diagnostic
/// (dump, preview, log) grade identically.
fn grade_one(mut r: f32, mut g: f32, mut b: f32, saturation: f32, gamma: f32) -> [f32; 3] {
    if saturation != 1.0 {
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        r = y + (r - y) * saturation;
        g = y + (g - y) * saturation;
        b = y + (b - y) * saturation;
    }
    r = r.clamp(0.0, 255.0);
    g = g.clamp(0.0, 255.0);
    b = b.clamp(0.0, 255.0);
    if gamma != 1.0 {
        r = 255.0 * (r / 255.0).powf(gamma);
        g = 255.0 * (g / 255.0).powf(gamma);
        b = 255.0 * (b / 255.0).powf(gamma);
    }
    [r, g, b]
}

fn to_u8(c: [f32; 3]) -> [u8; 3] {
    [(c[0] + 0.5) as u8, (c[1] + 0.5) as u8, (c[2] + 0.5) as u8]
}

/// Grade a color for display, float 0..255 in, u8 out.
pub fn grade_color(c: [f32; 3], saturation: f32, gamma: f32) -> [u8; 3] {
    to_u8(grade_one(c[0], c[1], c[2], saturation, gamma))
}

/// The per-LED colors as bytes for the wire.
pub fn shape_colors(vals: &[[f32; 3]], saturation: f32, gamma: f32) -> Vec<[u8; 3]> {
    vals.iter()
        .map(|&c| grade_color(c, saturation, gamma))
        .collect()
}

/// Grade a packed RGB frame in place-equivalent, for the dump: it lets
/// a saved frame preview what the knobs do, at full spatial detail.
pub fn grade_rgb8(rgb: &[u8], saturation: f32, gamma: f32) -> Vec<u8> {
    if saturation == 1.0 && gamma == 1.0 {
        return rgb.to_vec();
    }
    rgb.as_chunks::<3>()
        .0
        .iter()
        .flat_map(|p| {
            to_u8(grade_one(
                p[0] as f32,
                p[1] as f32,
                p[2] as f32,
                saturation,
                gamma,
            ))
        })
        .collect()
}

/// The subsampled frame as packed RGB, for --dump.
pub fn sampled_rgb(f: &Frame) -> (Vec<u8>, usize, usize) {
    let sw = sampled_count(f.width, f.step);
    let sh = sampled_count(f.height, f.step);
    let mut out = Vec::with_capacity(sw * sh * 3);
    for sy in 0..sh {
        for sx in 0..sw {
            let p = pixel(f, sx, sy);
            out.push(p[0] as u8);
            out.push(p[1] as u8);
            out.push(p[2] as u8);
        }
    }
    (out, sw, sh)
}

#[cfg(test)]
mod tests {
    // The expected behavior below is fixed from the project's goal --
    // an ambient backlight where each LED shows the color of the screen
    // region physically nearest it -- and from the color-grading
    // definitions, never from this implementation. A failing assertion
    // means the code is wrong, not that the assertion should change.
    use super::*;

    // Build a BGRx frame (the capture's pixel format) whose screen
    // color at (x, y) is given by `rgb`, so tests state colors in RGB
    // and this encodes them the way the capture delivers them.
    fn frame(w: usize, h: usize, rgb: impl Fn(usize, usize) -> [u8; 3]) -> Vec<u8> {
        let mut data = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let [r, g, b] = rgb(x, y);
                let i = (y * w + x) * 4;
                data[i] = b;
                data[i + 1] = g;
                data[i + 2] = r;
            }
        }
        data
    }

    // ---- StartPoint / Direction -----------------------------------------

    #[test]
    fn start_point_parses_the_eight_codes() {
        let all = [
            ("bl", StartPoint::Bl),
            ("bc", StartPoint::Bc),
            ("br", StartPoint::Br),
            ("rc", StartPoint::Rc),
            ("tr", StartPoint::Tr),
            ("tc", StartPoint::Tc),
            ("tl", StartPoint::Tl),
            ("lc", StartPoint::Lc),
        ];
        for (code, sp) in all {
            assert_eq!(StartPoint::parse(code), Some(sp));
            assert_eq!(sp.name(), code, "name must round-trip");
        }
        assert_eq!(StartPoint::CODES.len(), 8);
    }

    #[test]
    fn start_point_rejects_anything_else() {
        for s in ["", "BL", "Bl", "b", "bottom-left", "bl ", "x", "cl", "cc"] {
            assert_eq!(StartPoint::parse(s), None, "{s:?} should not parse");
        }
    }

    #[test]
    fn direction_parses_and_names_both_codes() {
        assert_eq!(Direction::parse("clockwise"), Some(Direction::Clockwise));
        assert_eq!(
            Direction::parse("counterclockwise"),
            Some(Direction::Counterclockwise)
        );
        assert_eq!(Direction::parse("cw"), None);
        assert_eq!(Direction::Clockwise.name(), "clockwise");
        assert_eq!(Direction::Counterclockwise.name(), "counterclockwise");
        for code in Direction::CODES {
            assert!(Direction::parse(code).is_some(), "{code} must parse");
        }
    }

    // ---- Layout ---------------------------------------------------------

    #[test]
    fn layout_keeps_its_edges_and_sums_the_count() {
        let l = Layout::new(132, 75, 132, 75, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        assert_eq!(
            (l.top(), l.right(), l.bottom(), l.left()),
            (132, 75, 132, 75)
        );
        assert_eq!(l.count(), 414);
        assert_eq!(l.packet_len(), 414);
        assert_eq!(l.start(), StartPoint::Bl);
        assert_eq!(l.direction(), Direction::Clockwise);
        assert_eq!(l.offset(), 0);
    }

    #[test]
    fn layout_accepts_up_to_one_packet_and_rejects_beyond() {
        // 490 LEDs is the most a single DRGB packet carries.
        assert!(Layout::new(490, 0, 0, 0, StartPoint::Bl, Direction::Clockwise, 0).is_ok());
        assert!(Layout::new(123, 122, 123, 122, StartPoint::Bl, Direction::Clockwise, 0).is_ok()); // 490
        assert!(Layout::new(491, 0, 0, 0, StartPoint::Bl, Direction::Clockwise, 0).is_err());
        assert!(Layout::new(123, 123, 123, 122, StartPoint::Bl, Direction::Clockwise, 0).is_err()); // 491
    }

    #[test]
    fn layout_counts_a_positive_offset_in_the_packet_length() {
        // Packet length L = count + max(offset, 0), still capped at 490.
        assert!(Layout::new(489, 0, 0, 0, StartPoint::Bl, Direction::Clockwise, 1).is_ok()); // L=490
        assert!(Layout::new(460, 0, 0, 0, StartPoint::Bl, Direction::Clockwise, 30).is_ok()); // L=490
        assert!(Layout::new(490, 0, 0, 0, StartPoint::Bl, Direction::Clockwise, 1).is_err()); // L=491
        assert!(Layout::new(461, 0, 0, 0, StartPoint::Bl, Direction::Clockwise, 30).is_err()); // L=491
        // A negative offset rotates, it does not lengthen the packet.
        let l = Layout::new(490, 0, 0, 0, StartPoint::Bl, Direction::Clockwise, -30).unwrap();
        assert_eq!(l.packet_len(), 490);
        assert!(Layout::new(491, 0, 0, 0, StartPoint::Bl, Direction::Clockwise, -30).is_err());
    }

    #[test]
    fn layout_rejects_no_leds_but_allows_a_missing_edge() {
        assert!(Layout::new(0, 0, 0, 0, StartPoint::Bl, Direction::Clockwise, 0).is_err());
        // A three-sided strip (no bottom) is a legitimate setup.
        assert!(Layout::new(10, 5, 0, 5, StartPoint::Bl, Direction::Clockwise, 0).is_ok());
    }

    #[test]
    fn layout_rejects_a_midpoint_start_on_an_empty_edge() {
        // start=tc requires top >= 1, and likewise for the other
        // midpoints; corner starts need no extra constraint.
        assert!(Layout::new(0, 5, 5, 5, StartPoint::Tc, Direction::Clockwise, 0).is_err());
        assert!(Layout::new(5, 0, 5, 5, StartPoint::Rc, Direction::Clockwise, 0).is_err());
        assert!(Layout::new(5, 5, 0, 5, StartPoint::Bc, Direction::Clockwise, 0).is_err());
        assert!(Layout::new(5, 5, 5, 0, StartPoint::Lc, Direction::Clockwise, 0).is_err());
        assert!(Layout::new(5, 5, 0, 5, StartPoint::Bl, Direction::Clockwise, 0).is_ok());
        assert!(Layout::new(0, 5, 5, 5, StartPoint::Tl, Direction::Counterclockwise, 0).is_ok());
        assert!(Layout::new(5, 0, 5, 5, StartPoint::Tc, Direction::Clockwise, 0).is_ok());
        let err = Layout::new(0, 5, 5, 5, StartPoint::Tc, Direction::Clockwise, 0).unwrap_err();
        assert!(matches!(
            err,
            LayoutError::MidpointOnEmptyEdge { start: "tc" }
        ));
    }

    // ---- edge_means -----------------------------------------------------

    #[test]
    fn edge_means_of_a_solid_frame_is_that_color_everywhere() {
        // Every zone of every edge must report the one screen color,
        // which also proves the BGRx bytes are read back as RGB.
        let data = frame(40, 40, |_, _| [10, 20, 30]);
        let f = Frame {
            data: &data,
            width: 40,
            height: 40,
            stride: 160,
            step: 1,
        };
        let l = Layout::new(4, 3, 4, 3, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        let e = edge_means(&f, 0.1, &l);
        for zones in [&e.top, &e.right, &e.bottom, &e.left] {
            for c in zones {
                assert_eq!(c.map(|v| v as u8), [10, 20, 30]);
            }
        }
    }

    #[test]
    fn edge_means_orders_top_and_bottom_left_to_right() {
        // Left half red, right half blue. The horizontal edges run
        // left-to-right, so zone 0 is red and zone 1 is blue; the left
        // edge sees only red, the right edge only blue.
        let (red, blue) = ([200u8, 0, 0], [0u8, 0, 200]);
        let data = frame(40, 40, |x, _| {
            if x < 20 {
                red
            } else {
                blue
            }
        });
        let f = Frame {
            data: &data,
            width: 40,
            height: 40,
            stride: 160,
            step: 1,
        };
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        let e = edge_means(&f, 0.1, &l);
        assert_eq!(e.top[0].map(|v| v as u8), red);
        assert_eq!(e.top[1].map(|v| v as u8), blue);
        assert_eq!(e.bottom[0].map(|v| v as u8), red);
        assert_eq!(e.bottom[1].map(|v| v as u8), blue);
        assert_eq!(e.left[0].map(|v| v as u8), red);
        assert_eq!(e.left[1].map(|v| v as u8), red);
        assert_eq!(e.right[0].map(|v| v as u8), blue);
        assert_eq!(e.right[1].map(|v| v as u8), blue);
    }

    #[test]
    fn edge_means_orders_left_and_right_top_to_bottom() {
        // Top half green, bottom half red. The vertical edges run
        // top-to-bottom, so zone 0 is green and zone 1 is red.
        let (green, red) = ([0u8, 200, 0], [200u8, 0, 0]);
        let data = frame(40, 40, |_, y| {
            if y < 20 {
                green
            } else {
                red
            }
        });
        let f = Frame {
            data: &data,
            width: 40,
            height: 40,
            stride: 160,
            step: 1,
        };
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        let e = edge_means(&f, 0.1, &l);
        assert_eq!(e.left[0].map(|v| v as u8), green);
        assert_eq!(e.left[1].map(|v| v as u8), red);
        assert_eq!(e.right[0].map(|v| v as u8), green);
        assert_eq!(e.right[1].map(|v| v as u8), red);
        assert_eq!(e.top[0].map(|v| v as u8), green);
        assert_eq!(e.top[1].map(|v| v as u8), green);
        assert_eq!(e.bottom[0].map(|v| v as u8), red);
        assert_eq!(e.bottom[1].map(|v| v as u8), red);
    }

    // ---- chain ----------------------------------------------------------

    // Two zones per edge, each tagged [edge_id, position, 0] so the
    // chained order can be read back. Edge ids: top 10, right 20,
    // bottom 30, left 40; position is the canonical index (top/bottom
    // left-to-right, left/right top-to-bottom).
    fn tagged_edges() -> EdgeColors {
        let e = |id: f32| vec![[id, 0.0, 0.0], [id, 1.0, 0.0]];
        EdgeColors {
            top: e(10.0),
            right: e(20.0),
            bottom: e(30.0),
            left: e(40.0),
        }
    }

    fn tags(chained: &[[f32; 3]]) -> Vec<(u8, u8)> {
        chained.iter().map(|c| (c[0] as u8, c[1] as u8)).collect()
    }

    #[test]
    fn chain_clockwise_from_bottom_left() {
        // Clockwise from bottom-left, seen from the front: up the left
        // edge, across the top, down the right, back across the bottom.
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        let got = tags(&chain(&tagged_edges(), &l));
        assert_eq!(
            got,
            vec![
                (40, 1),
                (40, 0), // left edge, bottom to top
                (10, 0),
                (10, 1), // top edge, left to right
                (20, 0),
                (20, 1), // right edge, top to bottom
                (30, 1),
                (30, 0), // bottom edge, right to left
            ],
        );
    }

    #[test]
    fn chain_counterclockwise_from_bottom_left() {
        // Counterclockwise from bottom-left: along the bottom, up the
        // right, across the top, down the left.
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Counterclockwise, 0).unwrap();
        let got = tags(&chain(&tagged_edges(), &l));
        assert_eq!(
            got,
            vec![
                (30, 0),
                (30, 1), // bottom edge, left to right
                (20, 1),
                (20, 0), // right edge, bottom to top
                (10, 1),
                (10, 0), // top edge, right to left
                (40, 0),
                (40, 1), // left edge, top to bottom
            ],
        );
    }

    #[test]
    fn chain_clockwise_from_top_left() {
        // Clockwise from top-left: across the top, down the right,
        // across the bottom, up the left.
        let l = Layout::new(2, 2, 2, 2, StartPoint::Tl, Direction::Clockwise, 0).unwrap();
        let got = tags(&chain(&tagged_edges(), &l));
        assert_eq!(
            got,
            vec![
                (10, 0),
                (10, 1), // top edge, left to right
                (20, 0),
                (20, 1), // right edge, top to bottom
                (30, 1),
                (30, 0), // bottom edge, right to left
                (40, 1),
                (40, 0), // left edge, bottom to top
            ],
        );
    }

    #[test]
    fn chain_length_matches_the_led_count() {
        let l = Layout::new(4, 3, 4, 3, StartPoint::Br, Direction::Clockwise, 0).unwrap();
        let e = EdgeColors {
            top: vec![[0.0; 3]; 4],
            right: vec![[0.0; 3]; 3],
            bottom: vec![[0.0; 3]; 4],
            left: vec![[0.0; 3]; 3],
        };
        assert_eq!(chain(&e, &l).len(), l.count());
    }

    // The 16 start x direction combinations against the spec's index
    // table. base (2 zones per edge) = [L1,L0,T0,T1,R0,R1,B1,B0].
    #[test]
    fn chain_matches_the_sixteen_combination_truth_table() {
        let base: Vec<(u8, u8)> = vec![
            (40, 1),
            (40, 0),
            (10, 0),
            (10, 1),
            (20, 0),
            (20, 1),
            (30, 1),
            (30, 0),
        ];
        // The spec table, written out literally: (nl, nt, nr, nb) = (2, 2, 2, 2).
        let s = |start: StartPoint, cw: bool| -> usize {
            let (nl, nt, nr, nb, count) = (2isize, 2, 2, 2, 8);
            let s = match (start, cw) {
                (StartPoint::Bl, true) => 0,
                (StartPoint::Bl, false) => count - 1,
                (StartPoint::Tl, true) => nl,
                (StartPoint::Tl, false) => nl - 1,
                (StartPoint::Tr, true) => nl + nt,
                (StartPoint::Tr, false) => nl + nt - 1,
                (StartPoint::Br, true) => nl + nt + nr,
                (StartPoint::Br, false) => nl + nt + nr - 1,
                (StartPoint::Lc, true) => nl / 2,
                (StartPoint::Lc, false) => nl - 1 - nl / 2,
                (StartPoint::Tc, true) => nl + nt / 2,
                (StartPoint::Tc, false) => nl + nt - 1 - nt / 2,
                (StartPoint::Rc, true) => nl + nt + nr / 2,
                (StartPoint::Rc, false) => nl + nt + nr - 1 - nr / 2,
                (StartPoint::Bc, true) => nl + nt + nr + nb / 2,
                (StartPoint::Bc, false) => count - 1 - nb / 2,
            };
            s.rem_euclid(count) as usize
        };
        for start in [
            StartPoint::Bl,
            StartPoint::Bc,
            StartPoint::Br,
            StartPoint::Rc,
            StartPoint::Tr,
            StartPoint::Tc,
            StartPoint::Tl,
            StartPoint::Lc,
        ] {
            for cw in [true, false] {
                let dir = if cw {
                    Direction::Clockwise
                } else {
                    Direction::Counterclockwise
                };
                let l = Layout::new(2, 2, 2, 2, start, dir, 0).unwrap();
                let got = tags(&chain(&tagged_edges(), &l));
                let s = s(start, cw);
                let want: Vec<(u8, u8)> = (0..8)
                    .map(|i| {
                        if cw {
                            base[(s + i) % 8]
                        } else {
                            base[(s + 8 - i) % 8]
                        }
                    })
                    .collect();
                assert_eq!(got, want, "start {} clockwise={cw}", start.name());
            }
        }
    }

    #[test]
    fn chain_from_top_center_clockwise_starts_mid_top() {
        // start=tc, clockwise, 2 per edge: s = nl + nt/2 = 3, so the
        // chain starts at T1 and wraps around.
        let l = Layout::new(2, 2, 2, 2, StartPoint::Tc, Direction::Clockwise, 0).unwrap();
        let got = tags(&chain(&tagged_edges(), &l));
        assert_eq!(
            got,
            vec![
                (10, 1),
                (20, 0),
                (20, 1),
                (30, 1),
                (30, 0),
                (40, 1),
                (40, 0),
                (10, 0),
            ],
        );
    }

    #[test]
    fn chain_midpoint_splits_an_odd_edge() {
        // left=5, start=lc: clockwise s = floor(5/2) = 2, so the chain
        // starts at L2; counterclockwise s = 5-1-2 = 2 as well but walks
        // the other way.
        let e = EdgeColors {
            top: vec![[10.0, 0.0, 0.0]],
            right: vec![[20.0, 0.0, 0.0]],
            bottom: vec![[30.0, 0.0, 0.0]],
            left: (0..5).map(|i| [40.0, i as f32, 0.0]).collect(),
        };
        let l = Layout::new(1, 1, 1, 5, StartPoint::Lc, Direction::Clockwise, 0).unwrap();
        assert_eq!(
            tags(&chain(&e, &l)),
            vec![
                (40, 2),
                (40, 1),
                (40, 0),
                (10, 0),
                (20, 0),
                (30, 0),
                (40, 4),
                (40, 3),
            ],
        );
        let l = Layout::new(1, 1, 1, 5, StartPoint::Lc, Direction::Counterclockwise, 0).unwrap();
        // Counterclockwise from lc walks down the left edge, then along
        // the bottom, up the right, across the top: base[(2-i) mod 8].
        assert_eq!(
            tags(&chain(&e, &l)),
            vec![
                (40, 2),
                (40, 3),
                (40, 4),
                (30, 0),
                (20, 0),
                (10, 0),
                (40, 0),
                (40, 1),
            ],
        );
    }

    // ---- apply_offset ---------------------------------------------------

    fn chain8() -> Vec<[f32; 3]> {
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        chain(&tagged_edges(), &l)
    }

    #[test]
    fn offset_zero_returns_the_chain_unchanged() {
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        let c = chain8();
        assert_eq!(apply_offset(&c, &l), c);
        assert_eq!(apply_offset(&c, &l).len(), l.count());
    }

    #[test]
    fn positive_offset_pads_the_head_with_black() {
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 3).unwrap();
        let c = chain8();
        let got = apply_offset(&c, &l);
        assert_eq!(got.len(), 8 + 3);
        assert_eq!(&got[0..3], &[[0.0; 3]; 3]);
        assert_eq!(&got[3..], &c[..]);
    }

    #[test]
    fn negative_offset_rotates_the_chain() {
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, -3).unwrap();
        let c = chain8();
        let got = apply_offset(&c, &l);
        assert_eq!(got.len(), 8);
        for i in 0..8 {
            assert_eq!(got[i], c[(i + 3) % 8], "i={i}");
        }
    }

    #[test]
    fn apply_offset_builds_the_wire_packet_from_graded_colors() {
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 2).unwrap();
        let graded: Vec<[u8; 3]> = (0..8).map(|i| [i as u8, 0, 0]).collect();
        let got = apply_offset(&graded, &l);
        assert_eq!(got.len(), 10);
        assert_eq!(&got[0..2], &[[0u8; 3]; 2]);
        assert_eq!(&got[2..], &graded[..]);
    }

    // ---- color grading --------------------------------------------------

    #[test]
    fn grading_is_identity_at_the_defaults() {
        assert_eq!(
            shape_colors(&[[10.0, 20.0, 30.0]], 1.0, 1.0),
            vec![[10, 20, 30]]
        );
        assert_eq!(grade_color([200.0, 100.0, 50.0], 1.0, 1.0), [200, 100, 50]);
    }

    #[test]
    fn saturation_leaves_a_gray_unchanged() {
        // A gray has every channel equal to its own luma, so no
        // saturation gain can move it, whatever the luma weights are.
        assert_eq!(
            grade_color([100.0, 100.0, 100.0], 0.0, 1.0),
            [100, 100, 100]
        );
        assert_eq!(
            grade_color([100.0, 100.0, 100.0], 5.0, 1.0),
            [100, 100, 100]
        );
    }

    #[test]
    fn zero_saturation_makes_gray() {
        // Fully desaturated, every channel collapses to the shared luma.
        let [r, g, b] = grade_color([200.0, 100.0, 50.0], 0.0, 1.0);
        assert_eq!((r, g), (g, b));
    }

    #[test]
    fn saturation_above_one_widens_the_spread() {
        // A channel above the luma rises, one below it falls.
        let [r, _g, b] = grade_color([200.0, 100.0, 60.0], 1.5, 1.0);
        assert!(r > 200, "channel above luma should rise, got {r}");
        assert!(b < 60, "channel below luma should fall, got {b}");
    }

    #[test]
    fn gamma_below_one_brightens_and_above_one_darkens() {
        // out = 255 * (v/255)^gamma. For v = 100: gamma 0.5 -> 160,
        // gamma 2.0 -> 39. Computed from the definition, not the code.
        assert_eq!(
            grade_color([100.0, 100.0, 100.0], 1.0, 0.5),
            [160, 160, 160]
        );
        assert_eq!(grade_color([100.0, 100.0, 100.0], 1.0, 2.0), [39, 39, 39]);
    }

    #[test]
    fn grading_clamps_out_of_range_channels() {
        // Saturation drives red past 255 and the others below 0; the
        // result must clamp, never wrap.
        assert_eq!(grade_color([250.0, 0.0, 0.0], 2.0, 1.0), [255, 0, 0]);
    }

    #[test]
    fn grade_rgb8_passes_through_at_the_defaults() {
        assert_eq!(
            grade_rgb8(&[1, 2, 3, 4, 5, 6], 1.0, 1.0),
            vec![1, 2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn grade_rgb8_grades_every_pixel() {
        // Two gray pixels under gamma 0.5: 100 -> 160, 50 -> 113.
        assert_eq!(
            grade_rgb8(&[100, 100, 100, 50, 50, 50], 1.0, 0.5),
            vec![160, 160, 160, 113, 113, 113],
        );
    }
}
