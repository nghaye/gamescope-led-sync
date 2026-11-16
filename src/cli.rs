//! Command-line parsing (clap derive), merged with the config file
//! into the validated Config the rest of the program reads. Precedence:
//! CLI flag > config file > built-in default (SPEC §4.1).

use {
    crate::{
        config,
        geometry::{Direction, Layout, StartPoint},
    },
    clap::{CommandFactory, FromArgMatches, Parser, error::ErrorKind, parser::ValueSource},
    std::path::PathBuf,
};

pub use crate::config::Config;

/// Ambient LED sync for gamescope sessions.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Config file (default: $XDG_CONFIG_HOME/gamescope-led-sync/config.toml,
    /// then /etc/gamescope-led-sync/config.toml, then built-in defaults)
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// WLED hostname or IP; omit for a dry run
    #[arg(long, value_name = "HOST")]
    wled: Option<String>,

    /// WLED UDP realtime port
    #[arg(long, default_value_t = 21324)]
    port: u16,

    /// Seconds WLED waits after the stream stops before returning to its preset
    #[arg(long = "timeout", value_name = "S", default_value_t = 2)]
    timeout_s: u8,

    /// Processing and send rate, in frames per second
    #[arg(long, default_value_t = 30.0)]
    fps: f64,

    /// Subsampled frame height for the color math
    #[arg(long, default_value_t = 240)]
    sample_height: u32,

    /// Edge depth each LED averages, as a fraction of screen height
    #[arg(long = "band", value_name = "FRAC", default_value_t = 0.08)]
    band_frac: f64,

    /// LED count, top edge
    #[arg(long, default_value_t = 132)]
    top: usize,

    /// LED count, right edge
    #[arg(long, default_value_t = 75)]
    right: usize,

    /// LED count, bottom edge
    #[arg(long, default_value_t = 132)]
    bottom: usize,

    /// LED count, left edge
    #[arg(long, default_value_t = 75)]
    left: usize,

    /// Start point holding the strip's first LED
    #[arg(long, value_parser = StartPoint::CODES, default_value = "bl")]
    start: String,

    /// Strip runs counterclockwise, seen from the front
    #[arg(long)]
    counterclockwise: bool,

    /// Saturation gain; above 1 compensates the muted HDR capture
    #[arg(long, default_value_t = 1.0)]
    saturation: f32,

    /// Gamma shaping; below 1 lifts crushed shadows
    #[arg(long, default_value_t = 1.0)]
    gamma: f32,

    /// Smoothing factor per frame; 1 disables it
    #[arg(long, default_value_t = 0.5)]
    smooth: f32,

    /// Process one frame and exit
    #[arg(long)]
    once: bool,

    /// With --once: save the captured frame to this PNG
    #[arg(long, value_name = "PNG")]
    dump: Option<String>,

    /// Draw the edge colors as ANSI bars on this terminal
    #[arg(long)]
    preview: bool,
}

/// Emit a clap-styled error and exit, for validation clap cannot express
/// in an attribute.
fn bail(message: impl std::fmt::Display) -> ! {
    Cli::command()
        .error(ErrorKind::ValueValidation, message)
        .exit()
}

pub fn parse() -> Config {
    let matches = Cli::command().get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(e) => e.exit(),
    };
    let on_cli = |name: &str| matches.value_source(name) == Some(ValueSource::CommandLine);

    let discovered = config::discover(cli.config.as_deref()).unwrap_or_else(|e| bail(e));
    match &discovered.path {
        Some(path) => tracing::info!("config: {} ({})", path.display(), discovered.source),
        None => tracing::info!("config: {}", discovered.source),
    }
    let mut cfg = discovered.config;
    tracing::info!("on_no_session: {}", cfg.on_no_session.name());

    // CLI flag > config file > built-in default: a flag set on the
    // command line overrides the file, which overrides the defaults.
    if on_cli("wled") {
        cfg.wled = cli.wled;
    }
    if on_cli("port") {
        cfg.port = cli.port;
    }
    if on_cli("timeout_s") {
        cfg.timeout_s = cli.timeout_s;
    }
    if on_cli("fps") {
        cfg.fps = cli.fps;
    }
    if on_cli("sample_height") {
        cfg.sample_height = cli.sample_height;
    }
    if on_cli("band_frac") {
        cfg.band_frac = cli.band_frac;
    }
    if on_cli("saturation") {
        cfg.saturation = cli.saturation;
    }
    if on_cli("gamma") {
        cfg.gamma = cli.gamma;
    }
    if on_cli("smooth") {
        cfg.smooth = cli.smooth;
    }

    let mut top = cfg.layout.top();
    let mut right = cfg.layout.right();
    let mut bottom = cfg.layout.bottom();
    let mut left = cfg.layout.left();
    if on_cli("top") {
        top = cli.top;
    }
    if on_cli("right") {
        right = cli.right;
    }
    if on_cli("bottom") {
        bottom = cli.bottom;
    }
    if on_cli("left") {
        left = cli.left;
    }
    let start = if on_cli("start") {
        StartPoint::parse(&cli.start).expect("clap validated --start")
    } else {
        cfg.layout.start()
    };
    let direction = if on_cli("counterclockwise") {
        Direction::Counterclockwise
    } else {
        cfg.layout.direction()
    };
    let offset = cfg.layout.offset();
    cfg.layout =
        Layout::new(top, right, bottom, left, start, direction, offset).unwrap_or_else(|e| bail(e));

    cfg.once = cli.once;
    cfg.dump = cli.dump;
    cfg.preview = cli.preview;

    cfg.validate().unwrap_or_else(|e| bail(e));
    cfg
}
