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
    #[error("start {start} is a midpoint and needs its edge to have at least 1 LED")]
    MidpointOnEmptyEdge { start: &'static str },
    #[error("gap {gap} exceeds the maximum seam hole of 50 LED pitches")]
    GapTooLarge { gap: usize },
}

/// LED counts per edge plus the strip's physical orientation. A Layout
/// only exists if the chain fits one DRGB packet (490 LEDs); the offset
/// is a circular shift of the loop and never changes the packet length,
/// and so does the gap: the seam hole has no LEDs, the packet stays
/// `count`.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    top: usize,
    right: usize,
    bottom: usize,
    left: usize,
    start: StartPoint,
    direction: Direction,
    offset: i32,
    gap: usize,
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
            gap: 0,
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

    /// The maximum seam hole (the Decky slider's max, §4.2): at most 50
    /// LED pitches between the strip's last and first LED.
    pub const MAX_GAP: usize = 50;

    /// Set the hole at the loop seam (ticket 14, D13): `gap` LED pitches
    /// between the strip's last and first LED. The hole lies on the seam
    /// edge and is skipped when sampling the screen (see `seam_hole`), so
    /// no LED shows the screen region behind the creux. The packet length
    /// stays `count` — the hole has no LEDs.
    pub fn with_gap(mut self, gap: usize) -> Result<Layout, LayoutError> {
        if gap > Self::MAX_GAP {
            return Err(LayoutError::GapTooLarge { gap });
        }
        self.gap = gap;
        Ok(self)
    }

    pub fn gap(&self) -> usize {
        self.gap
    }

    pub fn count(&self) -> usize {
        self.top + self.right + self.bottom + self.left
    }

    /// LEDs in the DRGB packet: always the whole chain. The strip is a
    /// loop (the last LED sits next to the first), so the offset only
    /// shifts it circularly, and the gap (D13) is a hole without LEDs,
    /// so neither ever lengthens the packet.
    pub fn packet_len(&self) -> usize {
        self.count()
    }

    /// The one-line layout summary, logged at startup and again on a
    /// reload that changed the layout (§7).
    pub fn summary(&self) -> String {
        format!(
            "top {}, right {}, bottom {}, left {} = {} LEDs, start {}, {}, offset {}, gap {}",
            self.top,
            self.right,
            self.bottom,
            self.left,
            self.count(),
            self.start.name(),
            self.direction.name(),
            self.offset,
            self.gap,
        )
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

/// Zones of a horizontal edge with a seam hole: the edge is divided into
/// `n + hole_len` equal zones and the `hole_len` zones starting at
/// `hole_start` (the creux) are dropped, so the `n` kept zones cover the
/// edge minus the hole — each LED samples the screen region physically in
/// front of it, not the region behind the creux.
fn zones_x_hole(
    f: &Frame,
    rows: Range<usize>,
    n: usize,
    sw: usize,
    hole: Option<(usize, usize)>,
) -> Vec<[f32; 3]> {
    match hole {
        None => zones_x(f, rows, n, sw),
        Some((h, g)) => {
            let mut all = zones_x(f, rows, n + g, sw);
            all.drain(h..h + g);
            all
        },
    }
}

/// Vertical-edge counterpart of [`zones_x_hole`].
fn zones_y_hole(
    f: &Frame,
    cols: Range<usize>,
    n: usize,
    sh: usize,
    hole: Option<(usize, usize)>,
) -> Vec<[f32; 3]> {
    match hole {
        None => zones_y(f, cols, n, sh),
        Some((h, g)) => {
            let mut all = zones_y(f, cols, n + g, sh);
            all.drain(h..h + g);
            all
        },
    }
}

/// The hole at the loop seam (ticket 14, D13), when `gap > 0`: the seam is
/// between the strip's last and first LED, i.e. between base entries
/// `b-1` and `b` of the chain's base list. Returns `(edge, hole_start,
/// gap)` where `hole_start` is in the seam edge's **extended** (`n + gap`)
/// canonical zone numbering (top/bottom left→right, left/right top→bottom).
/// None when `gap == 0` (the loop is closed, every zone is sampled).
fn seam_hole(l: &Layout) -> Option<(Edge, usize, usize)> {
    let gap = l.gap();
    if gap == 0 {
        return None;
    }
    let count = l.count();
    let s = start_index(l);
    // The seam boundary b in the base list: between base[b-1] and base[b].
    let b = match l.direction() {
        Direction::Clockwise => s,
        Direction::Counterclockwise => (s + 1) % count,
    };
    let nl = l.left();
    let nt = l.top();
    let nr = l.right();
    // Base runs: left [0, nl), top [nl, nl+nt), right, bottom. The hole
    // belongs to the run starting at (or containing) b. Top and right run
    // in canonical order in the base list; left and bottom are reversed.
    let (edge, run_start, n, reversed) = if b < nl {
        (Edge::Left, 0, nl, true)
    } else if b < nl + nt {
        (Edge::Top, nl, nt, false)
    } else if b < nl + nt + nr {
        (Edge::Right, nl + nt, nr, false)
    } else {
        (Edge::Bottom, nl + nt + nr, l.bottom(), true)
    };
    let off = b - run_start;
    let hole_start = if reversed {
        n - off
    } else {
        off
    };
    Some((edge, hole_start, gap))
}

/// Per-edge zone colors in canonical reading order: top and bottom run
/// left to right, left and right run top to bottom. With `gap > 0` the
/// seam edge is divided into `n + gap` zones and the `gap` hole zones at
/// the seam boundary (the creux between the strip's last and first LED)
/// are dropped, so each edge still yields exactly its LED count of zones.
pub fn edge_means(f: &Frame, band_frac: f64, l: &Layout) -> EdgeColors {
    let sw = sampled_count(f.width, f.step);
    let sh = sampled_count(f.height, f.step);
    let band = ((band_frac * sh as f64).round() as usize).clamp(1, sh);
    let band_w = band.min(sw);
    let hole = seam_hole(l);
    let hole_at = |e: Edge| match hole {
        Some((he, h, g)) if he == e => Some((h, g)),
        _ => None,
    };
    EdgeColors {
        top: zones_x_hole(f, 0..band, l.top, sw, hole_at(Edge::Top)),
        bottom: zones_x_hole(f, sh - band..sh, l.bottom, sw, hole_at(Edge::Bottom)),
        left: zones_y_hole(f, 0..band_w, l.left, sh, hole_at(Edge::Left)),
        right: zones_y_hole(f, sw - band_w..sw, l.right, sh, hole_at(Edge::Right)),
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

/// Which physical edge a zone belongs to, in the base list's order:
/// left edge, top edge, right edge, bottom edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Edge {
    Top = 0,
    Right = 1,
    Bottom = 2,
    Left = 3,
}

/// The edge a midpoint start point sits on — the loop's seam edge. None
/// for a corner start.
pub fn seam_edge(start: StartPoint) -> Option<Edge> {
    match start {
        StartPoint::Tc => Some(Edge::Top),
        StartPoint::Rc => Some(Edge::Right),
        StartPoint::Bc => Some(Edge::Bottom),
        StartPoint::Lc => Some(Edge::Left),
        _ => None,
    }
}

/// The edge tag of every chain position, first LED first — exactly the
/// ordering `chain()` produces (the base list's per-edge runs, rotated
/// or reversed per the start point and direction).
pub fn chain_edges(l: &Layout) -> Vec<Edge> {
    let mut base = Vec::with_capacity(l.count());
    base.extend(std::iter::repeat_n(Edge::Left, l.left()));
    base.extend(std::iter::repeat_n(Edge::Top, l.top()));
    base.extend(std::iter::repeat_n(Edge::Right, l.right()));
    base.extend(std::iter::repeat_n(Edge::Bottom, l.bottom()));
    let s = start_index(l);
    let count = l.count();
    (0..count)
        .map(|i| match l.direction() {
            Direction::Clockwise => base[(s + i) % count],
            Direction::Counterclockwise => base[(s + count - i) % count],
        })
        .collect()
}

/// A per-edge test pattern run through the same chain + offset math as
/// the live colors: `edge_colors[edge as usize]` for each LED. When the
/// start point is an edge midpoint, that seam edge is split in two at
/// the loop's seam: its head run (the beginning of the strip) keeps the
/// edge color, its tail run (the end of the strip, physically next to
/// the beginning) is painted `seam`, so the two halves of the seam edge
/// read differently — which is what makes the circular offset visible.
pub fn edge_pattern(l: &Layout, edge_colors: &[[u8; 3]; 4], seam: [u8; 3]) -> Vec<[u8; 3]> {
    let edges = chain_edges(l);
    let seam_e = seam_edge(l.start());
    // The head run of the seam edge is the chain's leading run of that
    // edge; its tail run sits at the chain's end.
    let head = match seam_e {
        Some(e) => edges.iter().take_while(|&&t| t == e).count(),
        None => 0,
    };
    let colored: Vec<[u8; 3]> = edges
        .iter()
        .enumerate()
        .map(|(i, &e)| {
            if Some(e) == seam_e && i >= head {
                seam
            } else {
                edge_colors[e as usize]
            }
        })
        .collect();
    apply_offset(&colored, l)
}

/// Build the DRGB packet from the chain. The strip is a loop — the last
/// LED is physically next to the first — so the offset is a circular
/// shift of the whole strip, whatever its sign: LED `i` gets
/// `chain[(i - offset) rem_euclid count]`. A positive offset shifts the
/// image toward higher LED indices (right along the strip), a negative
/// one toward lower indices (left), wrapping around the loop. The packet
/// always has `count` LEDs, so the offset — including a sign change —
/// never alters its length. Works on zones ([f32; 3]) or on graded
/// colors ([u8; 3]).
pub fn apply_offset<T: Copy>(chained: &[T], l: &Layout) -> Vec<T> {
    let count = l.count();
    let offset = l.offset() as isize;
    debug_assert_eq!(chained.len(), count);
    (0..count)
        .map(|i| chained[(i as isize - offset).rem_euclid(count as isize) as usize])
        .collect()
}

/// The color grading, on one pixel, in the locked order of §5.4:
/// saturation gain, then gamma, then the brightness multiplier, then
/// the clamp to 0..255 (the `as u8` cast in to_u8 saturates). All three
/// knobs are no-ops at 1.0. The saturation compensates the muted look
/// of gamescope's HDR-to-SDR capture; the gamma lifts its crushed
/// shadows. Kept in one place so the LED output and every diagnostic
/// (dump, preview, log) grade identically.
fn grade_one(
    mut r: f32,
    mut g: f32,
    mut b: f32,
    saturation: f32,
    gamma: f32,
    brightness: f32,
) -> [f32; 3] {
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
    [r * brightness, g * brightness, b * brightness]
}

fn to_u8(c: [f32; 3]) -> [u8; 3] {
    [(c[0] + 0.5) as u8, (c[1] + 0.5) as u8, (c[2] + 0.5) as u8]
}

/// Grade a color for display, float 0..255 in, u8 out.
pub fn grade_color(c: [f32; 3], saturation: f32, gamma: f32, brightness: f32) -> [u8; 3] {
    to_u8(grade_one(c[0], c[1], c[2], saturation, gamma, brightness))
}

/// The per-LED colors as bytes for the wire.
pub fn shape_colors(
    vals: &[[f32; 3]],
    saturation: f32,
    gamma: f32,
    brightness: f32,
) -> Vec<[u8; 3]> {
    vals.iter()
        .map(|&c| grade_color(c, saturation, gamma, brightness))
        .collect()
}

/// Grade a packed RGB frame in place-equivalent, for the dump: it lets
/// a saved frame preview what the knobs do, at full spatial detail.
pub fn grade_rgb8(rgb: &[u8], saturation: f32, gamma: f32, brightness: f32) -> Vec<u8> {
    if saturation == 1.0 && gamma == 1.0 && brightness == 1.0 {
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
                brightness,
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
    fn offset_never_changes_the_packet_length() {
        // The strip is a loop: the offset is a circular shift, so the
        // packet always carries the whole chain and only the count is
        // capped at 490.
        for offset in [-30, -1, 0, 1, 30] {
            let l =
                Layout::new(490, 0, 0, 0, StartPoint::Bl, Direction::Clockwise, offset).unwrap();
            assert_eq!(l.packet_len(), 490, "offset {offset}");
        }
        assert!(Layout::new(489, 0, 0, 0, StartPoint::Bl, Direction::Clockwise, 30).is_ok());
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

    #[test]
    fn gap_defaults_to_zero_and_is_bounded_at_50() {
        // The Decky slider's range, locked as the parameter's bound:
        // 0..=50 LED pitches at the loop seam.
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        assert_eq!(l.gap(), 0, "Layout::new starts with a closed loop");
        assert_eq!(
            l.packet_len(),
            l.count(),
            "the gap never lengthens the packet"
        );
        let l = l.with_gap(7).unwrap();
        assert_eq!(l.gap(), 7);
        assert_eq!(
            l.packet_len(),
            8,
            "the hole has no LEDs: packet stays count"
        );
        assert_eq!(l.count(), 8);
        assert!(l.clone().with_gap(50).is_ok(), "50 is the maximum");
        let err = l.clone().with_gap(51).unwrap_err();
        assert!(matches!(err, LayoutError::GapTooLarge { gap: 51 }));
        assert!(l.summary().contains("gap 7"), "{}", l.summary());
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

    // ---- seam hole (gap, ticket 14) --------------------------------------

    // 2 zones per edge, count = 8; base runs: left [0,2), top [2,4),
    // right [4,6), bottom [6,8). b = s (cw) or s+1 (ccw).
    fn layout2x2(start: StartPoint, dir: Direction, gap: usize) -> Layout {
        Layout::new(2, 2, 2, 2, start, dir, 0)
            .unwrap()
            .with_gap(gap)
            .unwrap()
    }

    #[test]
    fn seam_hole_is_none_when_gap_is_zero() {
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bc, Direction::Clockwise, 0).unwrap();
        assert_eq!(seam_hole(&l), None);
    }

    #[test]
    fn seam_hole_sits_at_the_seam_boundary_per_start_and_direction() {
        // bc cw: s = 2+2+2+1 = 7, b = 7, bottom run [6,8), off = 1,
        // reversed -> hole_start = nb - off = 1: the middle of the bottom.
        assert_eq!(
            seam_hole(&layout2x2(StartPoint::Bc, Direction::Clockwise, 3)),
            Some((Edge::Bottom, 1, 3))
        );
        // bc ccw: s = 8-1-1 = 6, b = 7 -> same hole (midpoint is
        // direction-independent).
        assert_eq!(
            seam_hole(&layout2x2(StartPoint::Bc, Direction::Counterclockwise, 3)),
            Some((Edge::Bottom, 1, 3))
        );
        // tc cw: s = 2+1 = 3, b = 3, top run [2,4), off = 1 -> middle of top.
        assert_eq!(
            seam_hole(&layout2x2(StartPoint::Tc, Direction::Clockwise, 2)),
            Some((Edge::Top, 1, 2))
        );
        // rc cw: s = 2+2+1 = 5, b = 5, right run [4,6), off = 1.
        assert_eq!(
            seam_hole(&layout2x2(StartPoint::Rc, Direction::Clockwise, 2)),
            Some((Edge::Right, 1, 2))
        );
        // lc cw: s = 1, b = 1, left run [0,2), off = 1, reversed ->
        // hole_start = nl - off = 1: middle of the left edge.
        assert_eq!(
            seam_hole(&layout2x2(StartPoint::Lc, Direction::Clockwise, 2)),
            Some((Edge::Left, 1, 2))
        );
        // bl cw: s = 0, b = 0 -> left run, off = 0, reversed ->
        // hole_start = nl = 2: the bottom end of the left edge (the corner).
        assert_eq!(
            seam_hole(&layout2x2(StartPoint::Bl, Direction::Clockwise, 2)),
            Some((Edge::Left, 2, 2))
        );
        // tl cw: s = 2, b = 2 -> top run, off = 0: the left end of the top.
        assert_eq!(
            seam_hole(&layout2x2(StartPoint::Tl, Direction::Clockwise, 2)),
            Some((Edge::Top, 0, 2))
        );
    }

    #[test]
    fn edge_means_skips_the_creux_on_the_seam_edge_only() {
        // 60 px wide; the bottom band is painted in 10 px blocks, block z
        // = [z, 0, 0], so a zone's mean color names the blocks it covers.
        let data = frame(60, 40, |x, _| [(x / 10) as u8, 0, 0]);
        let f = Frame {
            data: &data,
            width: 60,
            height: 40,
            stride: 240,
            step: 1,
        };
        // bottom = 4, gap = 2, start = bc: the bottom edge is divided into
        // 4+2 = 6 zones of 10 px and the 2 middle zones (blocks 2, 3 — the
        // creux) are dropped; the 4 kept zones are blocks 0, 1, 4, 5.
        let l = Layout::new(2, 2, 4, 2, StartPoint::Bc, Direction::Clockwise, 0)
            .unwrap()
            .with_gap(2)
            .unwrap();
        let e = edge_means(&f, 0.1, &l);
        assert_eq!(e.bottom.len(), 4, "the edge still yields its LED count");
        assert_eq!(
            e.bottom
                .iter()
                .map(|c| c.map(|v| v as u8))
                .collect::<Vec<_>>(),
            vec![[0, 0, 0], [1, 0, 0], [4, 0, 0], [5, 0, 0]],
            "the creux blocks 2 and 3 are not sampled"
        );
        // The other edges are untouched: 2 zones each over the full span.
        assert_eq!(e.top.len(), 2);
        assert_eq!(e.left.len(), 2);
        assert_eq!(e.right.len(), 2);
        // The chain's seam sits at the creux edges: bc cw starts at
        // B[nb/2-1] = block 1 and ends at B[nb/2] = block 4.
        let c = chain(&e, &l);
        assert_eq!(c[0].map(|v| v as u8), [1, 0, 0]);
        assert_eq!(c[l.count() - 1].map(|v| v as u8), [4, 0, 0]);
        // gap = 0: the same edge is divided into 4 zones of 15 px, mixing
        // neighbouring blocks — the creux is not skipped any more.
        let l0 = Layout::new(2, 2, 4, 2, StartPoint::Bc, Direction::Clockwise, 0).unwrap();
        let e0 = edge_means(&f, 0.1, &l0);
        assert_eq!(e0.bottom.len(), 4);
        // zone 2 covers px [30,45) = blocks 3 and 4 -> mean 3.33 -> 3.
        assert_eq!(e0.bottom[2].map(|v| v as u8), [3, 0, 0]);
        assert_ne!(e0.bottom, e.bottom, "gap 0 samples the whole edge");
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
    fn positive_offset_shifts_the_chain_toward_higher_indices() {
        // packet[i] = chain[(i - offset) mod count]: +3 moves the image
        // three LEDs forward along the strip, wrapping around the loop.
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 3).unwrap();
        let c = chain8();
        let got = apply_offset(&c, &l);
        assert_eq!(got.len(), 8, "the offset never lengthens the packet");
        for i in 0..8 {
            assert_eq!(got[i], c[(i + 8 - 3) % 8], "i={i}");
        }
    }

    #[test]
    fn negative_offset_shifts_the_chain_toward_lower_indices() {
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, -3).unwrap();
        let c = chain8();
        let got = apply_offset(&c, &l);
        assert_eq!(got.len(), 8);
        for i in 0..8 {
            assert_eq!(got[i], c[(i + 3) % 8], "i={i}");
        }
    }

    #[test]
    fn offset_beyond_the_count_wraps_around_the_loop() {
        // The last LED is next to the first: 10 mod 8 = 2, -10 mod 8 = 6.
        let c = chain8();
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 10).unwrap();
        let got = apply_offset(&c, &l);
        for i in 0..8 {
            assert_eq!(got[i], c[(i + 8 - 2) % 8], "offset +10, i={i}");
        }
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, -10).unwrap();
        let got = apply_offset(&c, &l);
        for i in 0..8 {
            assert_eq!(got[i], c[(i + 2) % 8], "offset -10, i={i}");
        }
    }

    #[test]
    fn apply_offset_builds_the_wire_packet_from_graded_colors() {
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 2).unwrap();
        let graded: Vec<[u8; 3]> = (0..8).map(|i| [i as u8, 0, 0]).collect();
        let got = apply_offset(&graded, &l);
        assert_eq!(got.len(), 8);
        for i in 0..8 {
            assert_eq!(got[i], graded[(i + 8 - 2) % 8], "i={i}");
        }
    }

    // ---- seam edge / edge_pattern ---------------------------------------

    #[test]
    fn seam_edge_is_the_midpoint_edge_only() {
        assert_eq!(seam_edge(StartPoint::Tc), Some(Edge::Top));
        assert_eq!(seam_edge(StartPoint::Rc), Some(Edge::Right));
        assert_eq!(seam_edge(StartPoint::Bc), Some(Edge::Bottom));
        assert_eq!(seam_edge(StartPoint::Lc), Some(Edge::Left));
        for corner in [
            StartPoint::Bl,
            StartPoint::Br,
            StartPoint::Tr,
            StartPoint::Tl,
        ] {
            assert_eq!(seam_edge(corner), None);
        }
    }

    #[test]
    fn chain_edges_matches_chain_ordering() {
        // Same base runs as chain(): left, top, right, bottom, rotated
        // per the start point and direction.
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        assert_eq!(
            chain_edges(&l),
            vec![
                Edge::Left,
                Edge::Left,
                Edge::Top,
                Edge::Top,
                Edge::Right,
                Edge::Right,
                Edge::Bottom,
                Edge::Bottom,
            ]
        );
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Counterclockwise, 0).unwrap();
        assert_eq!(
            chain_edges(&l),
            vec![
                Edge::Bottom,
                Edge::Bottom,
                Edge::Right,
                Edge::Right,
                Edge::Top,
                Edge::Top,
                Edge::Left,
                Edge::Left,
            ]
        );
    }

    #[test]
    fn edge_pattern_splits_a_midpoint_seam_edge_head_red_tail_seam() {
        // bc clockwise, 2 per edge: chain tags = [B,L,L,T,T,R,R,B]; the
        // head run [B] keeps bottom's color, the tail run [B] is the
        // seam color.
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bc, Direction::Clockwise, 0).unwrap();
        let colors = [[0, 1, 0], [0, 2, 0], [0, 3, 0], [0, 4, 0]]; // per Edge order T,R,B,L
        let got = edge_pattern(&l, &colors, [9, 9, 9]);
        assert_eq!(
            got,
            vec![
                [0, 3, 0], // head of the seam edge: bottom color
                [0, 4, 0],
                [0, 4, 0],
                [0, 1, 0],
                [0, 1, 0],
                [0, 2, 0],
                [0, 2, 0],
                [9, 9, 9], // tail of the seam edge: seam color
            ]
        );
    }

    #[test]
    fn edge_pattern_keeps_corner_starts_single_colored() {
        // A corner start has no seam edge: every zone keeps its edge color.
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        let colors = [[0, 1, 0], [0, 2, 0], [0, 3, 0], [0, 4, 0]];
        let got = edge_pattern(&l, &colors, [9, 9, 9]);
        assert_eq!(
            got,
            vec![
                [0, 4, 0],
                [0, 4, 0],
                [0, 1, 0],
                [0, 1, 0],
                [0, 2, 0],
                [0, 2, 0],
                [0, 3, 0],
                [0, 3, 0],
            ]
        );
        assert!(!got.contains(&[9, 9, 9]), "no seam color on a corner start");
    }

    // ---- color grading --------------------------------------------------

    #[test]
    fn grading_is_identity_at_the_defaults() {
        assert_eq!(
            shape_colors(&[[10.0, 20.0, 30.0]], 1.0, 1.0, 1.0),
            vec![[10, 20, 30]]
        );
        assert_eq!(
            grade_color([200.0, 100.0, 50.0], 1.0, 1.0, 1.0),
            [200, 100, 50]
        );
    }

    #[test]
    fn saturation_leaves_a_gray_unchanged() {
        // A gray has every channel equal to its own luma, so no
        // saturation gain can move it, whatever the luma weights are.
        assert_eq!(
            grade_color([100.0, 100.0, 100.0], 0.0, 1.0, 1.0),
            [100, 100, 100]
        );
        assert_eq!(
            grade_color([100.0, 100.0, 100.0], 5.0, 1.0, 1.0),
            [100, 100, 100]
        );
    }

    #[test]
    fn zero_saturation_makes_gray() {
        // Fully desaturated, every channel collapses to the shared luma.
        let [r, g, b] = grade_color([200.0, 100.0, 50.0], 0.0, 1.0, 1.0);
        assert_eq!((r, g), (g, b));
    }

    #[test]
    fn saturation_above_one_widens_the_spread() {
        // A channel above the luma rises, one below it falls.
        let [r, _g, b] = grade_color([200.0, 100.0, 60.0], 1.5, 1.0, 1.0);
        assert!(r > 200, "channel above luma should rise, got {r}");
        assert!(b < 60, "channel below luma should fall, got {b}");
    }

    #[test]
    fn gamma_below_one_brightens_and_above_one_darkens() {
        // out = 255 * (v/255)^gamma. For v = 100: gamma 0.5 -> 160,
        // gamma 2.0 -> 39. Computed from the definition, not the code.
        assert_eq!(
            grade_color([100.0, 100.0, 100.0], 1.0, 0.5, 1.0),
            [160, 160, 160]
        );
        assert_eq!(
            grade_color([100.0, 100.0, 100.0], 1.0, 2.0, 1.0),
            [39, 39, 39]
        );
    }

    #[test]
    fn grading_clamps_out_of_range_channels() {
        // Saturation drives red past 255 and the others below 0; the
        // result must clamp, never wrap.
        assert_eq!(grade_color([250.0, 0.0, 0.0], 2.0, 1.0, 1.0), [255, 0, 0]);
    }

    #[test]
    fn brightness_scales_after_gamma_not_before() {
        // gamma 0.5 lifts 100 to 160, then brightness 0.5 halves it to
        // 80. Applied before the gamma instead, 100 -> 50 -> 113.
        assert_eq!(
            grade_color([100.0, 100.0, 100.0], 1.0, 0.5, 0.5),
            [80, 80, 80]
        );
    }

    #[test]
    fn brightness_zero_blacks_everything() {
        assert_eq!(grade_color([200.0, 100.0, 50.0], 1.5, 0.8, 0.0), [0, 0, 0]);
    }

    #[test]
    fn full_order_saturation_then_gamma_then_brightness() {
        // saturation 0 collapses to the luma 117.65; gamma 2.0 maps it
        // to 255 * (117.65/255)^2 = 54.28; brightness 0.5 -> 27.14 -> 27.
        assert_eq!(
            grade_color([200.0, 100.0, 50.0], 0.0, 2.0, 0.5),
            [27, 27, 27]
        );
    }

    #[test]
    fn shape_colors_applies_brightness_to_the_wire_colors() {
        assert_eq!(
            shape_colors(&[[100.0; 3]], 1.0, 1.0, 0.5),
            vec![[50, 50, 50]]
        );
    }

    #[test]
    fn grade_rgb8_passes_through_at_the_defaults() {
        assert_eq!(
            grade_rgb8(&[1, 2, 3, 4, 5, 6], 1.0, 1.0, 1.0),
            vec![1, 2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn grade_rgb8_grades_every_pixel() {
        // Two gray pixels under gamma 0.5: 100 -> 160, 50 -> 113.
        assert_eq!(
            grade_rgb8(&[100, 100, 100, 50, 50, 50], 1.0, 0.5, 1.0),
            vec![160, 160, 160, 113, 113, 113],
        );
    }
}
