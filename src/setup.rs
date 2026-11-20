//! Interactive first-run setup (`--setup`): asks for the WLED host and
//! the strip layout, probes the controller over HTTP, plays a per-edge
//! test pattern on the strip, and writes the config file. The daemon
//! never starts in this mode; `run` returns the process exit code.

use {
    crate::{
        config::Config,
        geometry::{self, Direction, Layout, StartPoint},
        output::WledSender,
    },
    std::{
        fs,
        io::{self, BufRead, Read, Write},
        net::{TcpStream, ToSocketAddrs},
        path::{Path, PathBuf},
        time::{Duration, Instant},
    },
};

/// WLED's HTTP port, probed with `GET /json/info`.
const HTTP_PORT: u16 = 80;
/// Connect/read timeout of the WLED probe.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// DRGB timeout byte sent during the strip test, and how long the
/// pattern is held before the final black frame.
const TEST_TIMEOUT_S: u8 = 3;
const TEST_HOLD: Duration = Duration::from_secs(3);
const TEST_TICK: Duration = Duration::from_millis(250);

/// How the setup finished.
pub enum Outcome {
    Written(PathBuf),
    Quit,
}

/// Entry point of `--setup`, returning the process exit code: converse
/// on the terminal, with the real network probe and strip test,
/// targeting `explicit` (the `--config` path) or the default config
/// path (§4.1).
pub fn run(explicit: Option<&Path>) -> i32 {
    let path = target_path(explicit);
    // An existing file at the target prefills the prompts; a missing or
    // invalid one (setup's usual case) starts from the defaults.
    let current = Config::load(&path).ok();
    let mut io = Term;
    let mut probe = |host: &str| probe_wled(host);
    let mut strip = |host: &str, port: u16, l: &Layout| test_strip(host, port, l);
    match converse(&mut io, &mut probe, &mut strip, &path, current.as_ref()) {
        Ok(Outcome::Written(p)) => {
            tracing::info!("setup: wrote {}", p.display());
            io.say(&format!("config written to {}", p.display()));
            io.say("run `gamescope-led-sync` to start syncing");
            0
        },
        Ok(Outcome::Quit) => {
            io.say("setup aborted; no config file written");
            0
        },
        Err(e) => {
            io.say(&format!("setup failed: {e}"));
            1
        },
    }
}

/// The terminal: one prompt + one line in, one line out.
trait Io {
    /// Print `prompt` and read one trimmed line; Ok(None) on EOF.
    fn ask(&mut self, prompt: &str) -> io::Result<Option<String>>;
    fn say(&mut self, line: &str);
}

struct Term;

impl Io for Term {
    fn ask(&mut self, prompt: &str) -> io::Result<Option<String>> {
        let mut out = io::stdout();
        out.write_all(prompt.as_bytes())?;
        out.flush()?;
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line)? == 0 {
            return Ok(None);
        }
        Ok(Some(line.trim().to_string()))
    }

    fn say(&mut self, line: &str) {
        println!("{line}");
    }
}

/// Print `label` (with `[default]` appended when given) and read one
/// line. Ok(None) on EOF; an empty line is returned as-is and keeps the
/// default at the call site.
fn ask(io: &mut dyn Io, label: &str, default: Option<&str>) -> io::Result<Option<String>> {
    let prompt = match default {
        Some(d) => format!("{label} [{d}]: "),
        None => format!("{label}: "),
    };
    io.ask(&prompt)
}

/// "y"/"yes", case-insensitive.
fn is_yes(s: &str) -> bool {
    s.eq_ignore_ascii_case("y") || s.eq_ignore_ascii_case("yes")
}

/// Ask for the WLED host and probe it until it answers or the user
/// accepts it anyway. Ok(None) on EOF.
fn ask_host(
    io: &mut dyn Io,
    probe: &mut dyn FnMut(&str) -> Result<String, String>,
    current: Option<&str>,
) -> io::Result<Option<String>> {
    loop {
        let Some(line) = ask(io, "WLED host or IP", current)? else {
            return Ok(None);
        };
        let host = if line.is_empty() {
            match current {
                Some(c) => c.to_string(),
                None => {
                    io.say("a host is required");
                    continue;
                },
            }
        } else {
            line
        };
        match probe(&host) {
            Ok(info) => {
                io.say(&format!("reachable: {info}"));
                return Ok(Some(host));
            },
            Err(e) => {
                io.say(&format!("not reachable: {e}"));
                let Some(retry) = ask(io, "retry? [Y/n]", None)? else {
                    return Ok(None);
                };
                if retry.is_empty() || is_yes(&retry) {
                    continue;
                }
                return Ok(Some(host));
            },
        }
    }
}

const LAYOUT_FIELDS: [&str; 6] = ["top", "right", "bottom", "left", "start", "direction"];

fn field_label(field: &str, layout: &Layout) -> (String, String) {
    let default = match field {
        "top" => layout.top().to_string(),
        "right" => layout.right().to_string(),
        "bottom" => layout.bottom().to_string(),
        "left" => layout.left().to_string(),
        "start" => layout.start().name().to_string(),
        "direction" => layout.direction().name().to_string(),
        _ => String::new(),
    };
    let label = match field {
        "top" => "LEDs on the top edge",
        "right" => "LEDs on the right edge",
        "bottom" => "LEDs on the bottom edge",
        "left" => "LEDs on the left edge",
        "start" => "start point (bl bc br rc tr tc tl lc)",
        "direction" => "direction (clockwise counterclockwise)",
        _ => field,
    };
    (label.to_string(), default)
}

/// Re-ask one layout field until it parses and the whole layout
/// validates (§4.3 layout rules). Ok(None) on EOF.
fn ask_layout_field(
    io: &mut dyn Io,
    field: &str,
    layout: &Layout,
) -> io::Result<Option<Layout>> {
    loop {
        let (label, default) = field_label(field, layout);
        let Some(line) = ask(io, &label, Some(&default))? else {
            return Ok(None);
        };
        let raw = if line.is_empty() { default } else { line };
        let (mut top, mut right, mut bottom, mut left, mut start, mut direction) = (
            layout.top(),
            layout.right(),
            layout.bottom(),
            layout.left(),
            layout.start(),
            layout.direction(),
        );
        match field {
            "top" | "right" | "bottom" | "left" => match raw.parse::<usize>() {
                Ok(n) => match field {
                    "top" => top = n,
                    "right" => right = n,
                    "bottom" => bottom = n,
                    _ => left = n,
                },
                Err(_) => {
                    io.say(&format!("{raw:?} is not a non-negative integer"));
                    continue;
                },
            },
            "start" => match StartPoint::parse(&raw) {
                Some(s) => start = s,
                None => {
                    io.say(&format!(
                        "unknown start point {raw:?}; expected one of {}",
                        StartPoint::CODES.join(" ")
                    ));
                    continue;
                },
            },
            "direction" => match Direction::parse(&raw) {
                Some(d) => direction = d,
                None => {
                    io.say(&format!(
                        "unknown direction {raw:?}; expected one of {}",
                        Direction::CODES.join(" ")
                    ));
                    continue;
                },
            },
            _ => {},
        }
        match Layout::new(top, right, bottom, left, start, direction, layout.offset()) {
            Ok(l) => return Ok(Some(l)),
            Err(e) => {
                io.say(&format!("invalid layout: {e}"));
                continue;
            },
        }
    }
}

/// The summary menu: modify one setting, test the strip, write the
/// config, or quit. Ok(None) on EOF.
fn ask_menu(io: &mut dyn Io) -> io::Result<Option<String>> {
    ask(io, "choice: [m]odify, [t]est strip, [w]rite config, [q]uit", None)
}

fn ask_field_name(io: &mut dyn Io) -> io::Result<Option<String>> {
    loop {
        let Some(line) = ask(io, "which setting (wled top right bottom left start direction)", None)?
        else {
            return Ok(None);
        };
        if ["wled", "top", "right", "bottom", "left", "start", "direction"].contains(&line.as_str())
        {
            return Ok(Some(line));
        }
        io.say("unknown setting; expected one of: wled top right bottom left start direction");
    }
}

/// Render, validate, confirm, and write the config file. Returns false
/// when the user declines to overwrite an existing file.
fn write_config(io: &mut dyn Io, path: &Path, host: &str, layout: &Layout) -> io::Result<bool> {
    let text = render_toml(host, layout);
    // Round-trip through the real parser before touching the disk.
    Config::from_file_content(path, &text)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    if path.exists() {
        let Some(answer) =
            ask(io, &format!("{} exists; overwrite? [y/N]", path.display()), None)?
        else {
            return Ok(false);
        };
        if !is_yes(&answer) {
            io.say("not overwritten");
            return Ok(false);
        }
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, &text)?;
    Ok(true)
}

/// The whole conversation, injectable for tests: `io` is the terminal,
/// `probe` checks a WLED host, `strip` plays the test pattern.
fn converse(
    io: &mut dyn Io,
    probe: &mut dyn FnMut(&str) -> Result<String, String>,
    strip: &mut dyn FnMut(&str, u16, &Layout) -> Result<(), String>,
    path: &Path,
    current: Option<&Config>,
) -> io::Result<Outcome> {
    let defaults = Config::defaults();
    let mut host = current.and_then(|c| c.wled.clone());
    let port = current.map_or(defaults.port, |c| c.port);
    let mut layout = current.map_or_else(|| defaults.layout.clone(), |c| c.layout.clone());

    io.say("gamescope-led-sync interactive setup");
    io.say(&format!("config file: {}", path.display()));
    io.say("(Enter keeps the value in brackets; Ctrl-D quits)");

    let Some(h) = ask_host(io, probe, host.as_deref())? else {
        return Ok(Outcome::Quit);
    };
    host = Some(h);

    for field in LAYOUT_FIELDS {
        let Some(l) = ask_layout_field(io, field, &layout)? else {
            return Ok(Outcome::Quit);
        };
        layout = l;
    }

    loop {
        io.say("");
        io.say(&format!("wled:  {}:{port}", host.as_deref().unwrap_or("")));
        io.say(&format!("layout: {}", layout.summary()));
        let Some(choice) = ask_menu(io)? else {
            return Ok(Outcome::Quit);
        };
        match choice.as_str() {
            "m" | "modify" => {
                let Some(key) = ask_field_name(io)? else {
                    return Ok(Outcome::Quit);
                };
                if key == "wled" {
                    let Some(h) = ask_host(io, probe, host.as_deref())? else {
                        return Ok(Outcome::Quit);
                    };
                    host = Some(h);
                } else {
                    let Some(l) = ask_layout_field(io, &key, &layout)? else {
                        return Ok(Outcome::Quit);
                    };
                    layout = l;
                }
            },
            "t" | "test" => {
                io.say(
                    "lighting each edge in its own color for 3 s \
                     (top red, right green, bottom blue, left yellow), then black...",
                );
                match strip(host.as_deref().unwrap_or(""), port, &layout) {
                    Ok(()) => io.say("strip test done"),
                    Err(e) => io.say(&format!("strip test failed: {e}")),
                }
            },
            "w" | "write" => {
                if write_config(io, path, host.as_deref().unwrap_or(""), &layout)? {
                    return Ok(Outcome::Written(path.to_path_buf()));
                }
            },
            "q" | "quit" => return Ok(Outcome::Quit),
            _ => io.say("choose m (modify), t (test strip), w (write config), q (quit)"),
        }
    }
}

/// The config file `--setup` writes: only the keys setup asks for;
/// every other parameter stays at its built-in default (§4.2).
fn render_toml(host: &str, l: &Layout) -> String {
    format!(
        "# gamescope-led-sync configuration, written by `gamescope-led-sync --setup`.\n\
         # Every key is optional: anything left out falls back to the built-in\n\
         # default (see deploy/config.toml.example for the full schema).\n\
         # Value precedence: CLI flag > this file > built-in default.\n\
         \n\
         [wled]\n\
         host = {host:?}\n\
         \n\
         [layout]\n\
         top = {}\n\
         right = {}\n\
         bottom = {}\n\
         left = {}\n\
         start = {:?}\n\
         direction = {:?}\n",
        l.top(),
        l.right(),
        l.bottom(),
        l.left(),
        l.start().name(),
        l.direction().name(),
    )
}

/// Where `--setup` writes: the explicit `--config PATH`, else the first
/// discovery candidate (§4.1).
fn target_path(explicit: Option<&Path>) -> PathBuf {
    explicit.map_or_else(
        || default_config_path(std::env::var_os("XDG_CONFIG_HOME"), std::env::var_os("HOME")),
        Path::to_path_buf,
    )
}

fn default_config_path(xdg: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> PathBuf {
    if let Some(xdg) = xdg {
        PathBuf::from(xdg).join("gamescope-led-sync/config.toml")
    } else if let Some(home) = home {
        PathBuf::from(home).join(".config/gamescope-led-sync/config.toml")
    } else {
        PathBuf::from("/etc/gamescope-led-sync/config.toml")
    }
}

/// Probe a WLED controller over HTTP: TCP connect to port 80, then
/// `GET /json/info`, expecting a 200. Returns a short description
/// (status plus firmware version when present).
fn probe_wled(host: &str) -> Result<String, String> {
    probe_wled_port(host, HTTP_PORT)
}

fn probe_wled_port(host: &str, port: u16) -> Result<String, String> {
    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("cannot resolve {host}: {e}"))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("no address for {host}"));
    }
    let mut last = String::new();
    for addr in &addrs {
        match TcpStream::connect_timeout(addr, PROBE_TIMEOUT) {
            Ok(mut stream) => {
                stream.set_read_timeout(Some(PROBE_TIMEOUT)).map_err(|e| e.to_string())?;
                let request =
                    format!("GET /json/info HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
                stream.write_all(request.as_bytes()).map_err(|e| e.to_string())?;
                let mut response = Vec::new();
                stream.read_to_end(&mut response).map_err(|e| e.to_string())?;
                let text = String::from_utf8_lossy(&response);
                let status = text.lines().next().unwrap_or("").trim().to_string();
                if !status.contains(" 200") {
                    return Err(format!("{host}: HTTP status {status:?}"));
                }
                let version = extract_json_string(&text, "ver")
                    .map(|v| format!(" (WLED {v})"))
                    .unwrap_or_default();
                return Ok(format!("HTTP 200 on /json/info{version}"));
            },
            Err(e) => last = format!("{addr}: {e}"),
        }
    }
    Err(format!("cannot connect to {host}:{port}: {last}"))
}

/// Pull `"key":"value"` out of a JSON body without a JSON dependency.
fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":\"");
    let start = json.find(&pat)? + pat.len();
    let end = json[start..].find('"')?;
    Some(json[start..start + end].to_string())
}

/// One solid color per edge (top red, right green, bottom blue, left
/// yellow), chained and offset exactly like the live colors, so the
/// test also shows whether the counts and start/direction are right.
fn test_pattern(l: &Layout) -> Vec<[u8; 3]> {
    let edges = geometry::EdgeColors {
        top: vec![[255.0, 0.0, 0.0]; l.top()],
        right: vec![[0.0, 255.0, 0.0]; l.right()],
        bottom: vec![[0.0, 0.0, 255.0]; l.bottom()],
        left: vec![[255.0, 255.0, 0.0]; l.left()],
    };
    let chained = geometry::chain(&edges, l);
    geometry::apply_offset(&geometry::shape_colors(&chained, 1.0, 1.0, 1.0), l)
}

/// Send the test pattern for TEST_HOLD so WLED stays in realtime mode,
/// then one black frame to switch the strip off.
fn test_strip(host: &str, port: u16, l: &Layout) -> Result<(), String> {
    let mut sender = WledSender::new(host, port, TEST_TIMEOUT_S).map_err(|e| e.to_string())?;
    let colors = test_pattern(l);
    let deadline = Instant::now() + TEST_HOLD;
    while Instant::now() < deadline {
        sender.send(&colors);
        std::thread::sleep(TEST_TICK);
    }
    sender.send(&vec![[0u8; 3]; l.packet_len()]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        std::{
            collections::VecDeque,
            ffi::OsString,
            net::{TcpListener, UdpSocket},
            thread,
        },
    };

    /// A scripted terminal: answers are consumed in order, EOF once they
    /// run out; everything printed is recorded in `out`.
    struct Scripted {
        answers: VecDeque<String>,
        out: String,
    }

    impl Scripted {
        fn new(answers: &[&str]) -> Scripted {
            Scripted {
                answers: answers.iter().map(|s| s.to_string()).collect(),
                out: String::new(),
            }
        }
    }

    impl Io for Scripted {
        fn ask(&mut self, prompt: &str) -> io::Result<Option<String>> {
            self.out.push_str(prompt);
            self.out.push('\n');
            Ok(self.answers.pop_front())
        }

        fn say(&mut self, line: &str) {
            self.out.push_str(line);
            self.out.push('\n');
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gls-setup-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ok_probe(_: &str) -> Result<String, String> {
        Ok("HTTP 200 on /json/info".to_string())
    }

    fn no_strip(_: &str, _: u16, _: &Layout) -> Result<(), String> {
        Ok(())
    }

    #[test]
    fn rendered_config_parses_back_with_the_setup_values() {
        let l = Layout::new(86, 48, 86, 48, StartPoint::Bc, Direction::Clockwise, 0).unwrap();
        let text = render_toml("192.168.1.50", &l);
        let cfg = Config::from_file_content(Path::new("setup.toml"), &text).unwrap();
        assert_eq!(cfg.wled.as_deref(), Some("192.168.1.50"));
        assert_eq!(
            (cfg.layout.top(), cfg.layout.right(), cfg.layout.bottom(), cfg.layout.left()),
            (86, 48, 86, 48)
        );
        assert_eq!(cfg.layout.start(), StartPoint::Bc);
        assert_eq!(cfg.layout.direction(), Direction::Clockwise);
        // Everything setup does not ask for stays at the built-in default.
        let d = Config::defaults();
        assert_eq!(cfg.port, d.port);
        assert_eq!(cfg.timeout_s, d.timeout_s);
        assert_eq!(cfg.fps, d.fps);
        assert_eq!(cfg.layout.offset(), 0);
        assert_eq!(cfg.brightness, d.brightness);
        assert_eq!(cfg.on_no_session, d.on_no_session);
    }

    #[test]
    fn rendered_host_is_escaped_as_a_toml_string() {
        let l = Layout::new(1, 1, 1, 1, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        let text = render_toml("we\"ird\\host", &l);
        let cfg = Config::from_file_content(Path::new("setup.toml"), &text).unwrap();
        assert_eq!(cfg.wled.as_deref(), Some("we\"ird\\host"));
    }

    #[test]
    fn default_config_path_follows_the_lookup_order() {
        let p = default_config_path(Some(OsString::from("/xdg")), Some(OsString::from("/home/u")));
        assert_eq!(p, PathBuf::from("/xdg/gamescope-led-sync/config.toml"));
        let p = default_config_path(None, Some(OsString::from("/home/u")));
        assert_eq!(p, PathBuf::from("/home/u/.config/gamescope-led-sync/config.toml"));
        let p = default_config_path(None, None);
        assert_eq!(p, PathBuf::from("/etc/gamescope-led-sync/config.toml"));
    }

    #[test]
    fn test_pattern_colors_each_edge_and_follows_the_chain() {
        // Clockwise from bl, 2 LEDs per edge: left bottom-to-top, top
        // left-to-right, right top-to-bottom, bottom right-to-left.
        let (red, green, blue, yellow) = ([255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]);
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        assert_eq!(test_pattern(&l), vec![yellow, yellow, red, red, green, green, blue, blue]);
        // A positive offset circularly shifts the chain two LEDs
        // forward along the loop, so blue wraps to the head.
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 2).unwrap();
        assert_eq!(
            test_pattern(&l),
            vec![blue, blue, yellow, yellow, red, red, green, green]
        );
    }

    #[test]
    fn converse_asks_probes_tests_modifies_then_writes() {
        let dir = temp_dir("write");
        let path = dir.join("config.toml");
        let mut io = Scripted::new(&[
            "192.168.1.50", // host (probe answers OK)
            "", "", "", "", // keep the default counts 132/75/132/75
            "bc", "clockwise", // start, direction
            "t",               // test the strip
            "m", "top", "100", // modify top to 100
            "w",               // write (no existing file: no confirmation)
        ]);
        let mut probes = Vec::new();
        let mut probe = |host: &str| {
            probes.push(host.to_string());
            ok_probe(host)
        };
        let mut tested = Vec::new();
        let mut strip = |host: &str, port: u16, l: &Layout| {
            tested.push((host.to_string(), port, l.clone()));
            Ok(())
        };
        let outcome = converse(&mut io, &mut probe, &mut strip, &path, None).unwrap();
        assert!(matches!(outcome, Outcome::Written(p) if p == path));
        assert_eq!(probes, vec!["192.168.1.50".to_string()]);
        assert_eq!(tested.len(), 1);
        assert_eq!(tested[0].0, "192.168.1.50");
        assert_eq!(tested[0].1, 21324, "default port when no config exists");
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.wled.as_deref(), Some("192.168.1.50"));
        assert_eq!(cfg.layout.top(), 100, "the modification landed in the file");
        assert_eq!(cfg.layout.start(), StartPoint::Bc);
        assert_eq!(cfg.layout.count(), 382);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn converse_quits_without_writing() {
        let dir = temp_dir("quit");
        let path = dir.join("config.toml");
        let mut io = Scripted::new(&["10.0.0.8", "", "", "", "", "", "", "q"]);
        let outcome = converse(&mut io, &mut ok_probe, &mut no_strip, &path, None).unwrap();
        assert!(matches!(outcome, Outcome::Quit));
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn eof_aborts_the_setup_without_writing() {
        let dir = temp_dir("eof");
        let path = dir.join("config.toml");
        let mut io = Scripted::new(&[]);
        let outcome = converse(&mut io, &mut ok_probe, &mut no_strip, &path, None).unwrap();
        assert!(matches!(outcome, Outcome::Quit));
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreachable_host_is_accepted_once_the_user_declines_retry() {
        let dir = temp_dir("probe-fail");
        let path = dir.join("config.toml");
        let mut io = Scripted::new(&["bad.host", "n", "", "", "", "", "", "", "q"]);
        let mut calls = Vec::new();
        let mut probe = |host: &str| {
            calls.push(host.to_string());
            Err("no route to host".to_string())
        };
        let outcome = converse(&mut io, &mut probe, &mut no_strip, &path, None).unwrap();
        assert!(matches!(outcome, Outcome::Quit));
        assert_eq!(calls, vec!["bad.host".to_string()], "probed exactly once");
        assert!(io.out.contains("not reachable: no route to host"), "{}", io.out);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_invalid_count_is_rejected_then_fixed() {
        let l0 = Layout::new(132, 75, 132, 75, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        let mut io = Scripted::new(&["999", "abc", "100"]);
        let l = ask_layout_field(&mut io, "top", &l0).unwrap().unwrap();
        assert_eq!(l.top(), 100);
        assert!(io.out.contains("invalid layout"), "{}", io.out);
        assert!(io.out.contains("not a non-negative integer"), "{}", io.out);
    }

    #[test]
    fn an_unknown_start_code_is_rejected() {
        let l0 = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        let mut io = Scripted::new(&["xx", "tc"]);
        let l = ask_layout_field(&mut io, "start", &l0).unwrap().unwrap();
        assert_eq!(l.start(), StartPoint::Tc);
        assert!(io.out.contains("unknown start point"), "{}", io.out);
    }

    #[test]
    fn existing_file_is_not_overwritten_without_confirmation() {
        let dir = temp_dir("overwrite");
        let path = dir.join("config.toml");
        fs::write(&path, "[wled]\nhost = \"old\"\n").unwrap();
        let l = Layout::new(10, 5, 10, 5, StartPoint::Bl, Direction::Clockwise, 0).unwrap();

        let mut io = Scripted::new(&["n"]);
        assert!(!write_config(&mut io, &path, "new.host", &l).unwrap());
        assert_eq!(Config::load(&path).unwrap().wled.as_deref(), Some("old"));

        let mut io = Scripted::new(&["y"]);
        assert!(write_config(&mut io, &path, "new.host", &l).unwrap());
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.wled.as_deref(), Some("new.host"));
        assert_eq!(cfg.layout.count(), 30);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_config_creates_missing_parent_directories() {
        let dir = temp_dir("parents");
        let path = dir.join("nested/deeper/config.toml");
        let l = Layout::new(1, 1, 1, 1, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        let mut io = Scripted::new(&[]);
        assert!(write_config(&mut io, &path, "10.0.0.9", &l).unwrap());
        assert_eq!(Config::load(&path).unwrap().wled.as_deref(), Some("10.0.0.9"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn probe_speaks_http_to_a_loopback_wled_stub() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            assert!(String::from_utf8_lossy(&buf).contains("GET /json/info"));
            let body = r#"{"ver":"0.15.0","name":"stub"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        let info = probe_wled_port("127.0.0.1", port).unwrap();
        server.join().unwrap();
        assert!(info.contains("200") && info.contains("0.15.0"), "{info}");
    }

    #[test]
    fn probe_reports_a_refused_loopback_connection() {
        // Nothing listens on port 1 of loopback: the connect is refused.
        let err = probe_wled_port("127.0.0.1", 1).unwrap_err();
        assert!(err.contains("cannot connect"), "{err}");
    }

    #[test]
    fn probe_reports_a_non_200_status() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            stream
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });
        let err = probe_wled_port("127.0.0.1", port).unwrap_err();
        server.join().unwrap();
        assert!(err.contains("404"), "{err}");
    }

    #[test]
    fn extract_json_string_reads_a_quoted_value() {
        assert_eq!(
            extract_json_string(r#"{"ver":"0.15.0","name":"x"}"#, "ver"),
            Some("0.15.0".to_string())
        );
        assert_eq!(extract_json_string(r#"{"name":"x"}"#, "ver"), None);
        assert_eq!(extract_json_string(r#"{"ver":1}"#, "ver"), None);
    }

    #[test]
    fn strip_test_sends_the_pattern_then_black_over_loopback_udp() {
        // A loopback "WLED": the first datagrams must be the DRGB test
        // pattern, the last one all black.
        let wled = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = wled.local_addr().unwrap().port();
        // A 500 ms read timeout ends the receive loop shortly after the
        // final black frame (the pattern is sent every 250 ms).
        wled.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        let receiver = thread::spawn(move || {
            let mut packets = Vec::new();
            let mut buf = [0u8; 2048];
            while let Ok(n) = wled.recv(&mut buf) {
                packets.push(buf[..n].to_vec());
            }
            packets
        });
        let l = Layout::new(2, 2, 2, 2, StartPoint::Bl, Direction::Clockwise, 0).unwrap();
        test_strip("127.0.0.1", port, &l).unwrap();
        let packets = receiver.join().unwrap();
        assert!(packets.len() >= 2, "pattern packets plus the black frame");
        let pattern = test_pattern(&l);
        let mut expected = vec![2u8, TEST_TIMEOUT_S];
        for c in &pattern {
            expected.extend_from_slice(c);
        }
        assert_eq!(packets[0], expected, "first packet is the test pattern");
        for p in &packets[..packets.len() - 1] {
            assert_eq!(*p, expected, "the pattern is held");
        }
        let mut black = vec![2u8, TEST_TIMEOUT_S];
        black.extend(std::iter::repeat_n(0u8, l.packet_len() * 3));
        assert_eq!(packets.last().unwrap(), &black, "last packet is black");
    }
}
