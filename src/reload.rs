//! Live config reload (SPEC §7) and the WLED sender lifecycle (§6.0):
//! the daemon stats the config file every tick and, on change, swaps the
//! shared `Rc<RefCell<Config>>` atomically. A file that is invalid,
//! deleted, or unreadable keeps the previous config active. A sender
//! that cannot be created never exits the daemon: it is retried every
//! 5 s and on every reload.

use {
    crate::{config::Config, output::WledSender},
    std::{
        cell::{Cell, RefCell},
        fs,
        path::PathBuf,
        rc::Rc,
        time::{Duration, Instant, SystemTime},
    },
};

/// How often a failed WledSender creation is retried (§6.0).
pub const SENDER_RETRY: Duration = Duration::from_secs(5);

/// Stats the config file and reloads it when its mtime or length changes.
pub struct Reloader {
    path: Option<PathBuf>,
    sig: Option<(SystemTime, u64)>,
    had_file: bool,
}

fn sig_of(meta: &fs::Metadata) -> (SystemTime, u64) {
    (meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len())
}

impl Reloader {
    /// New reloader for `path`, pre-seeded with the file's current
    /// signature so the first poll does not reload what startup loaded.
    pub fn new(path: Option<PathBuf>) -> Reloader {
        let mut r = Reloader {
            path,
            sig: None,
            had_file: false,
        };
        if let Some(p) = &r.path
            && let Ok(meta) = fs::metadata(p)
        {
            r.sig = Some(sig_of(&meta));
            r.had_file = true;
        }
        r
    }

    /// Poll the file. Returns a fresh valid config when it changed;
    /// returns None when nothing changed, when the file is gone or
    /// unreadable (one debug log on the state change, no spam), or when
    /// it is invalid (one warn, the caller keeps the previous config).
    ///
    /// CLI-only flags (--dry-run, --once, --dump, --preview) are carried
    /// over from `current`: the file can never set them (§4.1).
    pub fn poll(&mut self, current: &Config) -> Option<Config> {
        let path = self.path.as_ref()?;
        match fs::metadata(path) {
            Ok(meta) => {
                let sig = sig_of(&meta);
                if self.sig == Some(sig) {
                    return None;
                }
                self.sig = Some(sig);
                self.had_file = true;
                match Config::load(path) {
                    Ok(mut cfg) => {
                        cfg.dry_run = current.dry_run;
                        cfg.once = current.once;
                        cfg.dump = current.dump.clone();
                        cfg.preview = current.preview;
                        Some(cfg)
                    },
                    Err(e) => {
                        tracing::warn!("{e}; keeping the previous config");
                        None
                    },
                }
            },
            Err(_) => {
                if self.had_file {
                    tracing::debug!("config file gone or unreadable; keeping the previous config");
                }
                self.had_file = false;
                self.sig = None;
                None
            },
        }
    }
}

/// Poll the reloader and, on a valid change, atomically swap the shared
/// config, log each changed field, and re-apply the sender. Returns the
/// new config when one was swapped in.
pub fn reload_if_changed(
    cfg: &Rc<RefCell<Config>>,
    sender: &Rc<RefCell<Option<WledSender>>>,
    reloader: &Rc<RefCell<Reloader>>,
) -> Option<Config> {
    let new = reloader.borrow_mut().poll(&cfg.borrow())?;
    let old = cfg.borrow().clone();
    *cfg.borrow_mut() = new.clone();
    log_changes(&old, &new);
    apply_sender(&old, &new, sender);
    Some(new)
}

/// Log every field that changed between two configs (§7).
pub fn log_changes(old: &Config, new: &Config) {
    if old.wled != new.wled {
        tracing::info!("wled.host: {:?} -> {:?}", old.wled, new.wled);
    }
    if old.port != new.port {
        tracing::info!("wled.port: {} -> {}", old.port, new.port);
    }
    if old.timeout_s != new.timeout_s {
        tracing::info!("wled.timeout_s: {} -> {}", old.timeout_s, new.timeout_s);
    }
    if old.fps != new.fps {
        tracing::info!("processing.fps: {} -> {}", old.fps, new.fps);
    }
    if old.sample_height != new.sample_height {
        tracing::info!(
            "processing.sample_height: {} -> {}",
            old.sample_height, new.sample_height
        );
    }
    if old.band_frac != new.band_frac {
        tracing::info!("processing.band_frac: {} -> {}", old.band_frac, new.band_frac);
    }
    if old.saturation != new.saturation {
        tracing::info!("processing.saturation: {} -> {}", old.saturation, new.saturation);
    }
    if old.gamma != new.gamma {
        tracing::info!("processing.gamma: {} -> {}", old.gamma, new.gamma);
    }
    if old.smooth != new.smooth {
        tracing::info!("processing.smooth: {} -> {}", old.smooth, new.smooth);
    }
    if old.brightness != new.brightness {
        tracing::info!("processing.brightness: {} -> {}", old.brightness, new.brightness);
    }
    if old.on_no_session != new.on_no_session {
        tracing::info!(
            "behavior.on_no_session: {} -> {}",
            old.on_no_session.name(),
            new.on_no_session.name()
        );
    }
    if old.node_name != new.node_name {
        tracing::info!("capture.node_name: {} -> {}", old.node_name, new.node_name);
    }
    if old.layout != new.layout {
        tracing::info!("layout: {}", new.layout.summary());
    }
}

/// Recreate the WLED sender when the endpoint changed; a new host that
/// does not resolve keeps the old sender (§7). `timeout_s` is carried
/// in every packet header, so it is updated in place otherwise.
pub fn apply_sender(old: &Config, new: &Config, sender: &Rc<RefCell<Option<WledSender>>>) {
    if old.wled == new.wled && old.port == new.port {
        if let Some(s) = sender.borrow_mut().as_mut() {
            s.set_timeout(new.timeout_s);
        }
        return;
    }
    if !new.sends() {
        *sender.borrow_mut() = None;
        tracing::info!("WLED host removed; running senderless");
        return;
    }
    let host = new.wled.as_deref().expect("sends() implies a host");
    match WledSender::new(host, new.port, new.timeout_s) {
        Ok(s) => {
            tracing::info!("WLED endpoint changed: {host}:{}", new.port);
            *sender.borrow_mut() = Some(s);
        },
        Err(e) => tracing::warn!("{e}; keeping the previous sender"),
    }
}

/// Create the sender if there is none, retrying at most every
/// SENDER_RETRY (§6.0: a creation failure never exits the daemon).
pub fn ensure_sender(
    cfg: &Config,
    sender: &Rc<RefCell<Option<WledSender>>>,
    last_try: &Cell<Instant>,
) {
    if sender.borrow().is_some() || !cfg.sends() {
        return;
    }
    let now = Instant::now();
    if now.duration_since(last_try.get()) < SENDER_RETRY {
        return;
    }
    last_try.set(now);
    let host = cfg.wled.as_deref().expect("sends() implies a host");
    match WledSender::new(host, cfg.port, cfg.timeout_s) {
        Ok(s) => {
            tracing::info!("WLED sender ready: {host}:{}", cfg.port);
            *sender.borrow_mut() = Some(s);
        },
        Err(e) => tracing::warn!("{e}; retrying every 5 s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gls-reload-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn no_change_at_startup_and_valid_change_is_picked_up() {
        let dir = tempdir("valid");
        let path = dir.join("config.toml");
        fs::write(&path, "[wled]\nhost = \"a\"\nport = 1\n").unwrap();
        let mut r = Reloader::new(Some(path.clone()));
        let cur = Config::load(&path).unwrap();
        assert!(r.poll(&cur).is_none(), "no change right after startup");
        fs::write(&path, "[wled]\nhost = \"b\"\nport = 2\n[processing]\nfps = 60\n").unwrap();
        let new = r.poll(&cur).expect("a changed file must reload");
        assert_eq!(new.wled.as_deref(), Some("b"));
        assert_eq!(new.port, 2);
        assert_eq!(new.fps, 60.0);
        assert!(r.poll(&cur).is_none(), "no second reload without a change");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn invalid_file_keeps_the_previous_config() {
        let dir = tempdir("invalid");
        let path = dir.join("config.toml");
        fs::write(&path, "[wled]\nport = 1\n").unwrap();
        let mut r = Reloader::new(Some(path.clone()));
        let cur = Config::load(&path).unwrap();
        fs::write(&path, "[wled]\nport = 0\n").unwrap();
        assert!(r.poll(&cur).is_none(), "an invalid file must not swap");
        assert!(r.poll(&cur).is_none(), "no warn spam on every tick");
        // Different length on purpose: the reload signature is mtime+len.
        fs::write(&path, "[wled]\nport = 3\n# ok\n").unwrap();
        let new = r.poll(&cur).expect("fixing the file reloads it");
        assert_eq!(new.port, 3);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn deleted_file_keeps_the_previous_config() {
        let dir = tempdir("deleted");
        let path = dir.join("config.toml");
        fs::write(&path, "[processing]\nfps = 45\n").unwrap();
        let mut r = Reloader::new(Some(path.clone()));
        let cur = Config::load(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert!(r.poll(&cur).is_none());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cli_only_flags_survive_a_reload() {
        let dir = tempdir("cli-only");
        let path = dir.join("config.toml");
        fs::write(&path, "[processing]\nfps = 30\n").unwrap();
        let mut r = Reloader::new(Some(path.clone()));
        let mut cur = Config::load(&path).unwrap();
        cur.dry_run = true;
        cur.once = true;
        cur.dump = Some("frame.png".to_string());
        cur.preview = true;
        // Different length on purpose: the reload signature is mtime+len.
        fs::write(&path, "[processing]\nfps = 90\n# tuned\n").unwrap();
        let new = r.poll(&cur).unwrap();
        assert!(new.dry_run && new.once && new.preview);
        assert_eq!(new.dump.as_deref(), Some("frame.png"));
        assert_eq!(new.fps, 90.0);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reload_if_changed_swaps_the_shared_config() {
        let dir = tempdir("swap");
        let path = dir.join("config.toml");
        fs::write(&path, "[processing]\nfps = 30\n").unwrap();
        let cfg = Rc::new(RefCell::new(Config::load(&path).unwrap()));
        let sender: Rc<RefCell<Option<WledSender>>> = Rc::new(RefCell::new(None));
        let reloader = Rc::new(RefCell::new(Reloader::new(Some(path.clone()))));
        assert!(reload_if_changed(&cfg, &sender, &reloader).is_none());
        // Different length on purpose: the reload signature is mtime+len.
        fs::write(&path, "[processing]\nfps = 90\n# tuned\n").unwrap();
        let new = reload_if_changed(&cfg, &sender, &reloader).expect("changed file swaps");
        assert_eq!(new.fps, 90.0);
        assert_eq!(cfg.borrow().fps, 90.0, "the shared config was swapped");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unresolvable_new_host_keeps_the_sender_untouched() {
        // apply_sender on an endpoint change must not exit and must
        // keep the previous sender when the new host does not resolve.
        let mut old = Config::defaults();
        old.wled = Some("old.invalid".to_string());
        let mut new = old.clone();
        new.wled = Some("new.invalid".to_string());
        let sender: Rc<RefCell<Option<WledSender>>> = Rc::new(RefCell::new(None));
        apply_sender(&old, &new, &sender);
        assert!(sender.borrow().is_none(), "no sender could be created");
    }
}
