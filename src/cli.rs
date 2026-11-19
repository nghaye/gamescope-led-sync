//! Command-line parsing (clap derive), merged with the config file
//! into the validated Config the rest of the program reads. Precedence:
//! CLI flag > config file > built-in default (SPEC §4.1).

use {
    crate::{
        config::{self, ConfigError},
        geometry::{Direction, Layout, StartPoint},
        setup,
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

    /// Interactive setup: ask for the WLED host and the strip layout,
    /// probe the controller, test the strip, write the config file
    /// (default lookup path, or --config PATH), and exit
    #[arg(long)]
    setup: bool,

    /// Never send anything to WLED, log only
    #[arg(long)]
    dry_run: bool,

    /// WLED hostname or IP (never contacted with --dry-run)
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

    /// Strip direction from the start point, seen from the front
    #[arg(long, value_parser = Direction::CODES, default_value = "clockwise")]
    direction: String,

    /// LED offset: positive sends that many black LEDs first, negative rotates
    #[arg(long, value_name = "N", default_value_t = 0)]
    offset: i32,

    /// Saturation gain; above 1 compensates the muted HDR capture
    #[arg(long, default_value_t = 1.0)]
    saturation: f32,

    /// Gamma shaping; below 1 lifts crushed shadows
    #[arg(long, default_value_t = 1.0)]
    gamma: f32,

    /// Smoothing factor per frame; 1 disables it
    #[arg(long, default_value_t = 0.5)]
    smooth: f32,

    /// Brightness multiplier, 0.0 to 1.0
    #[arg(long, default_value_t = 1.0)]
    brightness: f32,

    /// Start with the capture paused (overrides [capture] start_paused;
    /// the Decky plugin's «capture at startup» switch writes the file key)
    #[arg(long)]
    start_paused: bool,

    /// Process one frame and exit
    #[arg(long)]
    once: bool,

    /// With --once: save the captured frame to this PNG
    #[arg(long, value_name = "PNG")]
    dump: Option<String>,

    /// Draw the edge colors as ANSI bars on this terminal
    #[arg(long)]
    preview: bool,

    /// Send the per-segment test pattern (bottom red, left blue, top
    /// green, right yellow) to WLED until SIGTERM; used by the Decky
    /// plugin's configuration screen. Debug-only, never in the file
    #[arg(long)]
    preview_strip: bool,
}

/// Emit a clap-styled error and exit, for validation clap cannot express
/// in an attribute.
fn bail(message: impl std::fmt::Display) -> ! {
    Cli::command()
        .error(ErrorKind::ValueValidation, message)
        .exit()
}

/// The result of parsing: the merged, validated config, plus the config
/// file path (None when the built-in defaults are used) so the daemon
/// can reload it live (§7).
pub struct Parsed {
    pub config: Config,
    pub path: Option<PathBuf>,
}

pub fn parse() -> Parsed {
    let matches = Cli::command().get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(e) => e.exit(),
    };
    // --setup is a mode, not a flag: it never reaches the daemon loop.
    if cli.setup {
        std::process::exit(setup::run(cli.config.as_deref()));
    }
    let discovered = config::discover(cli.config.as_deref()).unwrap_or_else(|e| bail(e));
    match &discovered.path {
        Some(path) => tracing::info!("config: {} ({})", path.display(), discovered.source),
        None => tracing::info!("config: {}", discovered.source),
    }
    let cfg = discovered.config;
    tracing::info!("on_no_session: {}", cfg.on_no_session.name());

    let config = merge(&cli, &matches, cfg).unwrap_or_else(|e| bail(e));
    Parsed {
        config,
        path: discovered.path,
    }
}

/// Overlay the CLI flags on the discovered config. Precedence: CLI
/// flag > config file > built-in default (SPEC §4.1); a flag absent
/// from the command line keeps the file's or default's value.
fn merge(cli: &Cli, matches: &clap::ArgMatches, mut cfg: Config) -> Result<Config, ConfigError> {
    let on_cli = |name: &str| matches.value_source(name) == Some(ValueSource::CommandLine);

    if on_cli("wled") {
        cfg.wled = cli.wled.clone();
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
    if on_cli("brightness") {
        cfg.brightness = cli.brightness;
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
    let direction = if on_cli("direction") {
        Direction::parse(&cli.direction).expect("clap validated --direction")
    } else {
        cfg.layout.direction()
    };
    let offset = if on_cli("offset") {
        cli.offset
    } else {
        cfg.layout.offset()
    };
    cfg.layout = Layout::new(top, right, bottom, left, start, direction, offset)?;

    if on_cli("start_paused") {
        cfg.start_paused = cli.start_paused;
    }

    cfg.dry_run = cli.dry_run;
    cfg.once = cli.once;
    cfg.dump = cli.dump.clone();
    cfg.preview = cli.preview;
    cfg.preview_strip = cli.preview_strip;

    cfg.validate()?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli_from(args: &[&str]) -> (Cli, clap::ArgMatches) {
        let matches = Cli::command().try_get_matches_from(args).unwrap();
        let cli = Cli::from_arg_matches(&matches).unwrap();
        (cli, matches)
    }

    fn file_config(text: &str) -> Config {
        Config::from_file_content(&PathBuf::from("test.toml"), text).unwrap()
    }

    #[test]
    fn cli_flag_beats_file_beats_default() {
        let file = file_config(
            r#"
            [wled]
            host = "file-host"
            port = 1111
            [layout]
            top = 10
            direction = "counterclockwise"
            offset = 3
            [processing]
            fps = 60
            brightness = 0.25
            "#,
        );
        let (cli, matches) = cli_from(&[
            "gamescope-led-sync",
            "--wled",
            "cli-host",
            "--port",
            "2222",
            "--direction",
            "clockwise",
            "--offset",
            "9",
            "--brightness",
            "0.75",
            "--dry-run",
        ]);
        let cfg = merge(&cli, &matches, file).unwrap();
        assert_eq!(cfg.wled.as_deref(), Some("cli-host"), "CLI > file");
        assert_eq!(cfg.port, 2222, "CLI > file");
        assert_eq!(cfg.layout.direction(), Direction::Clockwise, "CLI > file");
        assert_eq!(cfg.layout.offset(), 9, "CLI > file");
        assert_eq!(cfg.brightness, 0.75, "CLI > file");
        assert_eq!(cfg.layout.top(), 10, "file > default");
        assert_eq!(cfg.fps, 60.0, "file > default");
        assert_eq!(cfg.timeout_s, 2, "default untouched");
        assert!(cfg.dry_run, "CLI-only flag");
        assert!(!cfg.sends(), "--dry-run never sends");
    }

    #[test]
    fn file_beats_default_when_no_flag_is_given() {
        let file = file_config(
            r#"
            [wled]
            host = "file-host"
            timeout_s = 9
            [layout]
            start = "bc"
            direction = "counterclockwise"
            offset = -5
            [processing]
            band_frac = 0.2
            "#,
        );
        let (cli, matches) = cli_from(&["gamescope-led-sync"]);
        let cfg = merge(&cli, &matches, file).unwrap();
        assert_eq!(cfg.wled.as_deref(), Some("file-host"));
        assert_eq!(cfg.timeout_s, 9);
        assert_eq!(cfg.layout.start(), StartPoint::Bc);
        assert_eq!(cfg.layout.direction(), Direction::Counterclockwise);
        assert_eq!(cfg.layout.offset(), -5);
        assert_eq!(cfg.band_frac, 0.2);
        assert_eq!(cfg.port, 21324, "default");
        assert!(!cfg.dry_run);
    }

    #[test]
    fn defaults_apply_when_neither_flag_nor_file_sets_a_value() {
        let (cli, matches) = cli_from(&["gamescope-led-sync"]);
        let cfg = merge(&cli, &matches, Config::defaults()).unwrap();
        let d = Config::defaults();
        assert_eq!(cfg.port, d.port);
        assert_eq!(cfg.layout.start(), StartPoint::Bl);
        assert_eq!(cfg.layout.direction(), Direction::Clockwise);
        assert_eq!(cfg.layout.offset(), 0);
        assert_eq!(cfg.brightness, 1.0);
        assert_eq!(cfg.fps, d.fps);
        assert!(cfg.wled.is_none());
        assert!(!cfg.dry_run);
    }

    #[test]
    fn invalid_cli_value_names_the_field() {
        let (cli, matches) = cli_from(&["gamescope-led-sync", "--port", "0"]);
        let err = merge(&cli, &matches, Config::defaults()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("wled.port") && msg.contains("1..=65535"), "{msg}");
    }

    #[test]
    fn cli_layout_error_surfaces_from_merge() {
        // count = 489 with offset = 2 -> packet length 491 > 490.
        let (cli, matches) = cli_from(&[
            "gamescope-led-sync",
            "--top",
            "489",
            "--right",
            "0",
            "--bottom",
            "0",
            "--left",
            "0",
            "--offset",
            "2",
        ]);
        let err = merge(&cli, &matches, Config::defaults()).unwrap_err();
        assert!(matches!(&err, ConfigError::Layout(_)), "{err}");
        assert!(err.to_string().contains("491"), "{err}");
    }

    #[test]
    fn direction_and_start_reject_unknown_codes() {
        for args in [
            ["gamescope-led-sync", "--direction", "sideways"],
            ["gamescope-led-sync", "--start", "xx"],
        ] {
            assert!(Cli::command().try_get_matches_from(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn counterclockwise_flag_is_gone() {
        let args = ["gamescope-led-sync", "--counterclockwise"];
        assert!(Cli::command().try_get_matches_from(args).is_err());
    }

    #[test]
    fn start_paused_flag_overrides_the_file() {
        let file = file_config("[capture]\nstart_paused = false\n");
        let (cli, matches) = cli_from(&["gamescope-led-sync", "--start-paused"]);
        assert!(merge(&cli, &matches, file).unwrap().start_paused);
        // Absent flag keeps the file's value.
        let file = file_config("[capture]\nstart_paused = true\n");
        let (cli, matches) = cli_from(&["gamescope-led-sync"]);
        assert!(merge(&cli, &matches, file).unwrap().start_paused);
        // And the built-in default is false.
        let (cli, matches) = cli_from(&["gamescope-led-sync"]);
        assert!(
            !merge(&cli, &matches, Config::defaults())
                .unwrap()
                .start_paused
        );
    }

    #[test]
    fn preview_strip_is_a_cli_only_flag() {
        let (cli, matches) = cli_from(&["gamescope-led-sync", "--preview-strip"]);
        let cfg = merge(&cli, &matches, Config::defaults()).unwrap();
        assert!(cfg.preview_strip);
        assert!(!Config::defaults().preview_strip);
        // The file cannot set it: unknown key.
        assert!(
            Config::from_file_content(
                &PathBuf::from("t.toml"),
                "[capture]\npreview_strip = true\n"
            )
            .is_err()
        );
    }

    #[test]
    fn setup_flag_parses_and_combines_with_config() {
        let (cli, _) = cli_from(&["gamescope-led-sync"]);
        assert!(!cli.setup);
        let (cli, _) = cli_from(&["gamescope-led-sync", "--setup"]);
        assert!(cli.setup);
        let (cli, _) = cli_from(&["gamescope-led-sync", "--setup", "--config", "/tmp/x.toml"]);
        assert!(cli.setup);
        assert_eq!(cli.config, Some(PathBuf::from("/tmp/x.toml")));
    }
}
