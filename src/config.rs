//! TOML configuration file: schema (SPEC §4.2), lookup order (§4.1),
//! built-in defaults, and the shared validator used by both the CLI
//! and the file (§4.3).

use {
    crate::geometry::{Direction, Layout, LayoutError, StartPoint},
    serde::Deserialize,
    std::{
        fs,
        path::{Path, PathBuf},
    },
};

/// What to send while no gamescope session is connected (D5, §6.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnNoSession {
    KeepLast,
    Black,
    Preset,
}

impl OnNoSession {
    pub fn parse(s: &str) -> Option<OnNoSession> {
        match s {
            "keep-last" => Some(OnNoSession::KeepLast),
            "black" => Some(OnNoSession::Black),
            "preset" => Some(OnNoSession::Preset),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            OnNoSession::KeepLast => "keep-last",
            OnNoSession::Black => "black",
            OnNoSession::Preset => "preset",
        }
    }
}

/// The runtime configuration. CLI-only flags (--config, --dry-run,
/// --once, --dump, --preview, --preview-strip) never appear in the file.
#[derive(Clone, Debug)]
pub struct Config {
    pub wled: Option<String>,
    pub port: u16,
    pub timeout_s: u8,
    pub fps: f64,
    pub sample_height: u32,
    pub band_frac: f64,
    pub layout: Layout,
    pub saturation: f32,
    pub gamma: f32,
    pub smooth: f32,
    pub brightness: f32,
    pub on_no_session: OnNoSession,
    pub node_name: String,
    /// Start with the capture paused (the Decky plugin's «capture at
    /// startup» switch, D12). Applied at startup and on live reload.
    pub start_paused: bool,
    pub dry_run: bool,
    pub once: bool,
    pub dump: Option<String>,
    pub preview: bool,
    pub preview_strip: bool,
}

/// Why a config file or a value was rejected (§4.3).
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read config file {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid config file {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("{field}: {value} violates {bound}")]
    Field {
        field: &'static str,
        value: String,
        bound: &'static str,
    },
    #[error("layout: {0}")]
    Layout(#[from] LayoutError),
}

fn field<T: std::fmt::Display>(name: &'static str, value: T, bound: &'static str) -> ConfigError {
    ConfigError::Field {
        field: name,
        value: value.to_string(),
        bound,
    }
}

// ---- file schema (§4.2): every key optional, unknown keys rejected ----

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FileConfig {
    pub wled: WledFile,
    pub layout: LayoutFile,
    pub processing: ProcessingFile,
    pub behavior: BehaviorFile,
    pub capture: CaptureFile,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct WledFile {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub timeout_s: Option<u8>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LayoutFile {
    pub top: Option<usize>,
    pub right: Option<usize>,
    pub bottom: Option<usize>,
    pub left: Option<usize>,
    pub start: Option<String>,
    pub direction: Option<String>,
    pub offset: Option<i32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ProcessingFile {
    pub fps: Option<f64>,
    pub sample_height: Option<u32>,
    pub band_frac: Option<f64>,
    pub saturation: Option<f32>,
    pub gamma: Option<f32>,
    pub smooth: Option<f32>,
    pub brightness: Option<f32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BehaviorFile {
    pub on_no_session: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CaptureFile {
    pub node_name: Option<String>,
    pub start_paused: Option<bool>,
}

/// A discovered config file and where it came from (§4.1).
pub struct Discovered {
    pub config: Config,
    pub path: Option<PathBuf>,
    pub source: &'static str,
}

/// Look up the config file: explicit path, then
/// $XDG_CONFIG_HOME/gamescope-led-sync/config.toml, then
/// /etc/gamescope-led-sync/config.toml, then built-in defaults.
///
/// An explicit path that is missing or unparseable is an error (the
/// CLI turns it into exit code 2); a discovered file that fails to
/// parse is warned about and skipped.
pub fn discover(explicit: Option<&Path>) -> Result<Discovered, ConfigError> {
    if let Some(path) = explicit {
        let config = Config::load(path)?;
        return Ok(Discovered {
            config,
            path: Some(path.to_path_buf()),
            source: "explicit --config",
        });
    }
    let mut candidates = Vec::new();
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        candidates.push(PathBuf::from(xdg).join("gamescope-led-sync/config.toml"));
    } else if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".config/gamescope-led-sync/config.toml"));
    }
    candidates.push(PathBuf::from("/etc/gamescope-led-sync/config.toml"));
    for candidate in candidates {
        if candidate.exists() {
            return match Config::load(&candidate) {
                Ok(config) => Ok(Discovered {
                    config,
                    path: Some(candidate),
                    source: "config file",
                }),
                Err(e) => {
                    tracing::warn!("ignoring {}: {e}", candidate.display());
                    break;
                },
            };
        }
    }
    tracing::warn!("no config file found; using built-in defaults");
    Ok(Discovered {
        config: Config::defaults(),
        path: None,
        source: "built-in defaults",
    })
}

impl Config {
    /// The built-in defaults used when no file provides a value.
    pub fn defaults() -> Config {
        Config {
            wled: None,
            port: 21324,
            timeout_s: 2,
            fps: 30.0,
            sample_height: 240,
            band_frac: 0.08,
            layout: Layout::new(132, 75, 132, 75, StartPoint::Bl, Direction::Clockwise, 0)
                .expect("built-in layout is valid"),
            saturation: 1.0,
            gamma: 1.0,
            smooth: 0.5,
            brightness: 1.0,
            on_no_session: OnNoSession::Preset,
            node_name: "gamescope".to_string(),
            start_paused: false,
            dry_run: false,
            once: false,
            dump: None,
            preview: false,
            preview_strip: false,
        }
    }

    /// Whether packets go out at all: `--dry-run` never sends, and
    /// with no WLED host there is nothing to send to (SPEC §4.4).
    pub fn sends(&self) -> bool {
        !self.dry_run && self.wled.is_some()
    }

    /// Read, parse, and validate a config file.
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_file_content(path, &text)
    }

    /// Parse and validate TOML text as if it came from `path`.
    pub fn from_file_content(path: &Path, text: &str) -> Result<Config, ConfigError> {
        let file: FileConfig = toml::from_str(text).map_err(|e| ConfigError::Parse {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        let mut cfg = Config::defaults();
        cfg.merge_file(&file)?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Overlay the file's values on the current config.
    fn merge_file(&mut self, f: &FileConfig) -> Result<(), ConfigError> {
        if let Some(host) = &f.wled.host {
            self.wled = Some(host.clone());
        }
        if let Some(port) = f.wled.port {
            self.port = port;
        }
        if let Some(timeout_s) = f.wled.timeout_s {
            self.timeout_s = timeout_s;
        }

        let l = &f.layout;
        let mut top = self.layout.top();
        let mut right = self.layout.right();
        let mut bottom = self.layout.bottom();
        let mut left = self.layout.left();
        if let Some(v) = l.top {
            top = v;
        }
        if let Some(v) = l.right {
            right = v;
        }
        if let Some(v) = l.bottom {
            bottom = v;
        }
        if let Some(v) = l.left {
            left = v;
        }
        let start = match &l.start {
            Some(s) => StartPoint::parse(s)
                .ok_or_else(|| field("layout.start", s, "one of bl bc br rc tr tc tl lc"))?,
            None => self.layout.start(),
        };
        let direction = match &l.direction {
            Some(s) => Direction::parse(s)
                .ok_or_else(|| field("layout.direction", s, "clockwise|counterclockwise"))?,
            None => self.layout.direction(),
        };
        let offset = l.offset.unwrap_or(self.layout.offset());
        self.layout = Layout::new(top, right, bottom, left, start, direction, offset)?;

        if let Some(v) = f.processing.fps {
            self.fps = v;
        }
        if let Some(v) = f.processing.sample_height {
            self.sample_height = v;
        }
        if let Some(v) = f.processing.band_frac {
            self.band_frac = v;
        }
        if let Some(v) = f.processing.saturation {
            self.saturation = v;
        }
        if let Some(v) = f.processing.gamma {
            self.gamma = v;
        }
        if let Some(v) = f.processing.smooth {
            self.smooth = v;
        }
        if let Some(v) = f.processing.brightness {
            self.brightness = v;
        }

        if let Some(s) = &f.behavior.on_no_session {
            self.on_no_session = OnNoSession::parse(s)
                .ok_or_else(|| field("behavior.on_no_session", s, "keep-last|black|preset"))?;
        }

        if let Some(s) = &f.capture.node_name {
            self.node_name = s.clone();
        }
        if let Some(v) = f.capture.start_paused {
            self.start_paused = v;
        }
        Ok(())
    }

    /// The one validator shared by the CLI and the file (§4.3): every
    /// bound of §4.2, the layout rules, and the enum codes. Errors
    /// name the field, the value, and the bound.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if let Some(host) = &self.wled
            && host.is_empty()
        {
            return Err(field("wled.host", host, "non-empty hostname or IP"));
        }
        if self.port == 0 {
            return Err(field("wled.port", self.port, "1..=65535"));
        }
        if self.timeout_s == 0 {
            return Err(field("wled.timeout_s", self.timeout_s, "1..=255"));
        }
        if !self.fps.is_finite() || !(1.0..=240.0).contains(&self.fps) {
            return Err(field("processing.fps", self.fps, "1..=240"));
        }
        if self.sample_height == 0 || self.sample_height > 16384 {
            return Err(field(
                "processing.sample_height",
                self.sample_height,
                "1..=16384",
            ));
        }
        if !self.band_frac.is_finite() || self.band_frac <= 0.0 || self.band_frac >= 1.0 {
            return Err(field("processing.band_frac", self.band_frac, "> 0 and < 1"));
        }
        if !self.saturation.is_finite() || self.saturation < 0.0 {
            return Err(field("processing.saturation", self.saturation, ">= 0"));
        }
        if !self.gamma.is_finite() || self.gamma <= 0.0 {
            return Err(field("processing.gamma", self.gamma, "> 0"));
        }
        if !self.smooth.is_finite() || self.smooth <= 0.0 {
            return Err(field(
                "processing.smooth",
                self.smooth,
                "> 0 (1.0 disables)",
            ));
        }
        if !self.brightness.is_finite() || !(0.0..=1.0).contains(&self.brightness) {
            return Err(field("processing.brightness", self.brightness, "0.0..=1.0"));
        }
        if self.node_name.is_empty() {
            return Err(field("capture.node_name", &self.node_name, "non-empty"));
        }
        let l = &self.layout;
        Layout::new(
            l.top(),
            l.right(),
            l.bottom(),
            l.left(),
            l.start(),
            l.direction(),
            l.offset(),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Config, ConfigError> {
        Config::from_file_content(Path::new("test.toml"), text)
    }

    fn err_of(text: &str) -> String {
        parse(text).unwrap_err().to_string()
    }

    #[test]
    fn full_file_lands_every_section() {
        let cfg = parse(
            r#"
            [wled]
            host = "wled.local"
            port = 21324
            timeout_s = 3
            [layout]
            top = 86
            right = 48
            bottom = 86
            left = 48
            start = "bc"
            direction = "counterclockwise"
            offset = 5
            [processing]
            fps = 60
            sample_height = 480
            band_frac = 0.2
            saturation = 1.5
            gamma = 0.8
            smooth = 1.0
            brightness = 0.5
            [behavior]
            on_no_session = "black"
            [capture]
            node_name = "gamescope-alt"
            start_paused = true
            "#,
        )
        .unwrap();
        assert_eq!(cfg.wled.as_deref(), Some("wled.local"));
        assert_eq!(cfg.port, 21324);
        assert_eq!(cfg.timeout_s, 3);
        assert_eq!(cfg.layout.count(), 268);
        assert_eq!(cfg.layout.start(), StartPoint::Bc);
        assert_eq!(cfg.layout.direction(), Direction::Counterclockwise);
        assert_eq!(cfg.layout.offset(), 5);
        assert_eq!(
            cfg.layout.packet_len(),
            268,
            "the offset never lengthens the packet"
        );
        assert_eq!(cfg.fps, 60.0);
        assert_eq!(cfg.sample_height, 480);
        assert_eq!(cfg.band_frac, 0.2);
        assert_eq!(cfg.saturation, 1.5);
        assert_eq!(cfg.gamma, 0.8);
        assert_eq!(cfg.smooth, 1.0);
        assert_eq!(cfg.brightness, 0.5);
        assert_eq!(cfg.on_no_session, OnNoSession::Black);
        assert_eq!(cfg.node_name, "gamescope-alt");
        assert!(cfg.start_paused);
    }

    #[test]
    fn empty_file_gives_the_builtin_defaults() {
        let cfg = parse("").unwrap();
        let d = Config::defaults();
        assert!(cfg.wled.is_none());
        assert_eq!(cfg.port, d.port);
        assert_eq!(cfg.timeout_s, d.timeout_s);
        assert_eq!(cfg.layout.count(), d.layout.count());
        assert_eq!(cfg.layout.start(), d.layout.start());
        assert_eq!(cfg.on_no_session, OnNoSession::Preset);
        assert_eq!(cfg.node_name, "gamescope");
        assert!(!cfg.start_paused);
        assert_eq!(cfg.brightness, 1.0);
    }

    #[test]
    fn start_paused_must_be_a_bool() {
        assert!(matches!(
            parse("[capture]\nstart_paused = \"yes\"\n"),
            Err(ConfigError::Parse { .. })
        ));
        assert!(parse("[capture]\nstart_paused = false\n").is_ok());
    }

    #[test]
    fn partial_file_overlays_only_the_keys_present() {
        let cfg = parse("[processing]\nfps = 60\n").unwrap();
        assert_eq!(cfg.fps, 60.0);
        assert_eq!(cfg.port, 21324);
        assert_eq!(cfg.layout.count(), 414);
    }

    #[test]
    fn unknown_key_in_a_section_is_a_hard_error() {
        let msg = err_of("[wled]\nhosts = \"typo\"\n");
        assert!(msg.contains("hosts"), "error must name the key: {msg}");
    }

    #[test]
    fn unknown_section_is_a_hard_error() {
        assert!(parse("[procesing]\nfps = 30\n").is_err());
    }

    #[test]
    fn wrong_toml_type_is_a_parse_error() {
        assert!(matches!(
            parse("[wled]\nport = -1\n"),
            Err(ConfigError::Parse { .. })
        ));
    }

    #[test]
    fn port_must_be_in_1_to_65535() {
        let msg = err_of("[wled]\nport = 0\n");
        assert!(
            msg.contains("wled.port") && msg.contains("1..=65535"),
            "{msg}"
        );
    }

    #[test]
    fn timeout_must_be_in_1_to_255() {
        let msg = err_of("[wled]\ntimeout_s = 0\n");
        assert!(msg.contains("wled.timeout_s"), "{msg}");
    }

    #[test]
    fn empty_host_is_rejected() {
        let msg = err_of("[wled]\nhost = \"\"\n");
        assert!(msg.contains("wled.host"), "{msg}");
    }

    #[test]
    fn fps_must_be_in_1_to_240() {
        assert!(err_of("[processing]\nfps = 0\n").contains("processing.fps"));
        assert!(err_of("[processing]\nfps = 241\n").contains("processing.fps"));
    }

    #[test]
    fn sample_height_must_be_in_1_to_16384() {
        assert!(err_of("[processing]\nsample_height = 0\n").contains("processing.sample_height"));
        assert!(
            err_of("[processing]\nsample_height = 16385\n").contains("processing.sample_height")
        );
    }

    #[test]
    fn band_frac_must_be_strictly_between_0_and_1() {
        assert!(err_of("[processing]\nband_frac = 0.0\n").contains("band_frac"));
        assert!(err_of("[processing]\nband_frac = 1.0\n").contains("band_frac"));
    }

    #[test]
    fn saturation_must_be_non_negative() {
        assert!(err_of("[processing]\nsaturation = -0.5\n").contains("saturation"));
    }

    #[test]
    fn gamma_and_smooth_must_be_positive() {
        assert!(err_of("[processing]\ngamma = 0.0\n").contains("gamma"));
        assert!(err_of("[processing]\nsmooth = 0.0\n").contains("smooth"));
    }

    #[test]
    fn brightness_must_be_in_0_to_1() {
        assert!(err_of("[processing]\nbrightness = -0.1\n").contains("brightness"));
        assert!(err_of("[processing]\nbrightness = 1.1\n").contains("brightness"));
    }

    #[test]
    fn start_must_be_a_known_code() {
        let msg = err_of("[layout]\nstart = \"xx\"\n");
        assert!(msg.contains("layout.start") && msg.contains("xx"), "{msg}");
    }

    #[test]
    fn direction_must_be_a_known_code() {
        let msg = err_of("[layout]\ndirection = \"sideways\"\n");
        assert!(msg.contains("layout.direction"), "{msg}");
    }

    #[test]
    fn on_no_session_must_be_a_known_code() {
        let msg = err_of("[behavior]\non_no_session = \"explode\"\n");
        assert!(msg.contains("behavior.on_no_session"), "{msg}");
    }

    #[test]
    fn node_name_must_be_non_empty() {
        assert!(err_of("[capture]\nnode_name = \"\"\n").contains("capture.node_name"));
    }

    #[test]
    fn zero_leds_rejected() {
        assert!(matches!(
            parse("[layout]\ntop = 0\nright = 0\nbottom = 0\nleft = 0\n"),
            Err(ConfigError::Layout(LayoutError::ZeroLeds))
        ));
    }

    #[test]
    fn count_beyond_490_rejected_whatever_the_offset() {
        // All four edges are set explicitly: unset edges keep the
        // built-in defaults and would trip the count bound first. The
        // offset is a circular shift: it never lengthens the packet.
        assert!(matches!(
            parse("[layout]\ntop = 491\nright = 0\nbottom = 0\nleft = 0\noffset = -2\n"),
            Err(ConfigError::Layout(LayoutError::TooManyLeds { count: 491 }))
        ));
        assert!(
            parse("[layout]\ntop = 489\nright = 0\nbottom = 0\nleft = 0\noffset = 30\n").is_ok()
        );
    }

    #[test]
    fn midpoint_start_on_empty_edge_rejected() {
        assert!(matches!(
            parse("[layout]\ntop = 0\nright = 5\nbottom = 5\nleft = 5\nstart = \"tc\"\n"),
            Err(ConfigError::Layout(LayoutError::MidpointOnEmptyEdge {
                start: "tc"
            }))
        ));
    }

    #[test]
    fn builtin_defaults_validate() {
        Config::defaults().validate().unwrap();
    }

    #[test]
    fn dry_run_never_sends_even_with_a_host() {
        let mut cfg = Config::defaults();
        cfg.wled = Some("wled.local".to_string());
        assert!(cfg.sends());
        cfg.dry_run = true;
        assert!(!cfg.sends(), "--dry-run must suppress all sending");
        cfg.dry_run = false;
        cfg.wled = None;
        assert!(!cfg.sends(), "no host means nothing to send to");
    }

    #[test]
    fn shipped_example_config_parses_and_validates() {
        // install.sh copies deploy/config.toml.example as the initial
        // user config, so it must always parse and validate against the
        // schema (SPEC §9, ticket 06).
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("deploy/config.toml.example");
        let cfg = Config::load(&path).expect("deploy/config.toml.example must be valid");
        assert_eq!(cfg.layout.count(), 268);
        assert_eq!(cfg.layout.start(), StartPoint::Bc);
        assert_eq!(cfg.layout.direction(), Direction::Clockwise);
    }

    #[test]
    fn missing_explicit_file_is_a_read_error() {
        assert!(matches!(
            Config::load(Path::new("/nonexistent/gamescope-led-sync/config.toml")),
            Err(ConfigError::Read { .. })
        ));
    }

    #[test]
    fn discover_explicit_path_loads_that_file() {
        let dir = std::env::temp_dir().join(format!("gls-cfg-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(&path, "[wled]\nhost = \"from-file\"\n").unwrap();
        let d = discover(Some(&path)).unwrap();
        assert_eq!(d.config.wled.as_deref(), Some("from-file"));
        assert_eq!(d.path.as_deref(), Some(path.as_path()));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn discover_falls_back_to_defaults_when_no_file_exists() {
        let dir = std::env::temp_dir().join(format!("gls-cfg-empty-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
        let d = discover(None).unwrap();
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
        assert!(d.path.is_none());
        assert_eq!(d.config.node_name, "gamescope");
        assert_eq!(d.config.layout.count(), 414);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn discover_uses_xdg_file_when_present() {
        let dir = std::env::temp_dir().join(format!("gls-cfg-xdg-{}", std::process::id()));
        let sub = dir.join("gamescope-led-sync");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("config.toml"), "[processing]\nfps = 45\n").unwrap();
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
        let d = discover(None).unwrap();
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
        assert_eq!(d.config.fps, 45.0);
        assert!(d.path.is_some());
        fs::remove_dir_all(&dir).unwrap();
    }
}
