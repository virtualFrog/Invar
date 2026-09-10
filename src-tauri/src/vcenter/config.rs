//! Connection settings.
//!
//! The config holds a *list* of vCenters from day one — every fetch path is an
//! aggregation over connections, so single-server is just the one-element case.
//!
//! ## What is and is not in the file
//!
//! `config.json` holds hosts, usernames and the certificate policy. It does
//! **not** hold passwords: those go to the OS credential store (see
//! [`super::secrets`]). The `password` field below is
//! `skip_serializing`, which does double duty — it keeps secrets out of the
//! file, and it keeps them out of the JSON that `get_config` hands to the
//! webview, so a rendering bug or an injected script cannot read back a stored
//! vSphere admin password.
//!
//! Passwords still *deserialize*, for two reasons: the settings dialog posts a
//! whole `AppConfig` back when it saves, and a `config.json` written by an
//! older version has cleartext passwords in it that [`load`] migrates into the
//! credential store on first read.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct VCenterConnection {
    /// Hostname or IP, no scheme. `https://` is assumed.
    pub host: String,
    pub username: String,
    /// Never written to disk and never serialized to the frontend. Populated
    /// from the credential store by [`resolve`], or from the settings dialog
    /// when the user types a new one.
    #[serde(default, skip_serializing)]
    pub password: String,
    /// Accept a certificate that does not validate.
    ///
    /// Defaults to **false**: verify. A vCenter inventory tool carries admin
    /// credentials, and an earlier version pinned this to `true` in the
    /// settings UI with no way to turn it off, so every connection this app
    /// ever made was open to interception. Lab vCenters with self-signed certs
    /// are a real case, so the escape hatch stays — it is just a decision the
    /// operator makes per connection rather than one made for them.
    #[serde(default)]
    pub skip_cert_verify: bool,
    /// Serialized to the frontend only, so the settings dialog can show
    /// "stored" instead of an empty password box and can tell an unset
    /// password from one it simply is not allowed to see.
    #[serde(default, skip_deserializing)]
    pub has_stored_password: bool,
}

impl VCenterConnection {
    /// A connection with no password and the safe certificate policy. Test
    /// helper and the shape the settings dialog starts from.
    pub fn new(host: &str, username: &str) -> Self {
        Self {
            host: host.into(),
            username: username.into(),
            password: String::new(),
            skip_cert_verify: false,
            has_stored_password: false,
        }
    }

    pub fn with_password(mut self, password: &str) -> Self {
        self.password = password.into();
        self
    }

    pub fn trusting_invalid_certs(mut self) -> Self {
        self.skip_cert_verify = true;
        self
    }

    pub fn base_url(&self) -> String {
        let host = self
            .host
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        format!("https://{host}")
    }

    /// Identity for the session cache: host *and* user, so two accounts on the
    /// same vCenter — or the same account on two vCenters — never evict each other.
    pub fn cache_key(&self) -> String {
        format!("{}|{}", self.base_url(), self.username)
    }

    /// A label safe to show in the UI and to put in the `VI SDK Server` column.
    pub fn label(&self) -> String {
        self.base_url().trim_start_matches("https://").to_string()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub connections: Vec<VCenterConnection>,
}

pub fn config_path(app_dir: PathBuf) -> PathBuf {
    app_dir.join("config.json")
}

/// The bundle identifier, and so the name of the settings directory.
pub const APP_ID: &str = "ch.soultec.invar";

/// Where the desktop app keeps its settings, computed the way Tauri computes
/// `app_config_dir` for [`APP_ID`].
///
/// Duplicated rather than borrowed from Tauri on purpose: `invar-export` and
/// the Cargo examples are not Tauri applications and have no `AppHandle`, but
/// they must read the same `config.json` the GUI writes, or scheduling an
/// export would mean configuring the same vCenters twice.
pub fn default_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library").join("Application Support").join(APP_ID))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join(APP_ID))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|c| c.join(APP_ID))
    }
}

/// [`default_dir`] as a config file path, with a message worth reading when the
/// environment is too bare to have a home directory at all.
pub fn default_path() -> Result<PathBuf, String> {
    default_dir()
        .map(config_path)
        .ok_or_else(|| "could not determine a settings directory; pass --config".to_string())
}

/// Read the config, migrating any cleartext passwords it still contains.
///
/// The returned connections carry no passwords — call [`resolve`] for that.
/// Migration is deliberately one-way and quiet: an old file is read, its
/// secrets are moved into the credential store, and the file is rewritten
/// without them. If the credential store is unavailable the passwords are left
/// where they are rather than being destroyed, and the caller finds them again
/// next time.
pub fn load(path: &PathBuf) -> Result<AppConfig, String> {
    let mut cfg = read(path)?;
    let legacy = cfg.connections.iter().any(|c| !c.password.is_empty());
    if legacy && migrate(&cfg).is_ok() {
        let _ = write(path, &cfg);
    }
    for c in &mut cfg.connections {
        c.has_stored_password = !c.password.is_empty()
            || super::secrets::load(&c.label(), &c.username).ok().flatten().is_some();
        c.password.clear();
    }
    Ok(cfg)
}

/// The connections with their passwords filled in, ready to log in with.
///
/// Order of resolution, and the reason for it:
///
/// 1. `INVAR_PASSWORD_<n>`, 1-based over the connection list (`INVAR_PASSWORD`
///    is accepted as an alias for the first). A container or a systemd unit
///    has no credential store, and this is how it supplies one.
/// 2. The OS credential store.
///
/// A connection with no password resolvable either way is returned as an error
/// naming the host, not skipped. Silently dropping a vCenter is how an
/// inventory tool produces a short list that looks complete.
pub fn resolve(cfg: &AppConfig) -> Result<Vec<VCenterConnection>, String> {
    let mut out = Vec::with_capacity(cfg.connections.len());
    let mut missing = Vec::new();
    for (i, conn) in cfg.connections.iter().enumerate() {
        let mut conn = conn.clone();
        conn.has_stored_password = false;
        let from_env = std::env::var(format!("INVAR_PASSWORD_{}", i + 1))
            .ok()
            .or_else(|| if i == 0 { std::env::var("INVAR_PASSWORD").ok() } else { None });
        conn.password = match from_env {
            Some(p) => p,
            None => match super::secrets::load(&conn.label(), &conn.username)? {
                Some(p) => p,
                None => {
                    missing.push(conn.label());
                    continue;
                }
            },
        };
        out.push(conn);
    }
    if !missing.is_empty() {
        return Err(format!(
            "no password stored for {}. Open Settings and enter it, or set INVAR_PASSWORD_<n>.",
            missing.join(", ")
        ));
    }
    Ok(out)
}

/// Persist the config, and sync the credential store to match.
///
/// A connection whose password arrives empty keeps whatever is already stored:
/// the settings dialog cannot show an existing password, so it cannot resend
/// one, and treating "blank" as "erase it" would wipe a working credential
/// every time someone changed a hostname.
///
/// Credentials for connections that were removed are deleted, so uninstalling a
/// vCenter from the list does not leave its password behind in the Keychain.
pub fn save(path: &PathBuf, cfg: &AppConfig) -> Result<(), String> {
    let previous = read(path).unwrap_or_default();

    for conn in &cfg.connections {
        if !conn.password.is_empty() {
            super::secrets::store(&conn.label(), &conn.username, &conn.password)?;
        }
    }

    let kept: Vec<String> = cfg
        .connections
        .iter()
        .map(|c| super::secrets::account(&c.label(), &c.username))
        .collect();
    for old in &previous.connections {
        let account = super::secrets::account(&old.label(), &old.username);
        if !kept.contains(&account) {
            let _ = super::secrets::forget(&old.label(), &old.username);
        }
    }

    write(path, cfg)
}

/// Read and parse the file, with a missing file meaning "no connections yet".
fn read(path: &PathBuf) -> Result<AppConfig, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("config.json is not valid: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AppConfig::default()),
        Err(e) => Err(format!("could not read {}: {e}", path.display())),
    }
}

/// The on-disk shape, stated explicitly rather than falling out of whichever
/// serde attributes `VCenterConnection` happens to carry.
///
/// Without this, `has_stored_password` would be written into `config.json` on
/// every save: it is serialized for the frontend's benefit and skipped on
/// deserialize, so the file would grow a field that is always `false` and means
/// nothing to anyone editing the file by hand.
#[derive(Serialize)]
struct FileConnection<'a> {
    host: &'a str,
    username: &'a str,
    skip_cert_verify: bool,
}

#[derive(Serialize)]
struct FileConfig<'a> {
    connections: Vec<FileConnection<'a>>,
}

fn write(path: &PathBuf, cfg: &AppConfig) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let on_disk = FileConfig {
        connections: cfg
            .connections
            .iter()
            .map(|c| FileConnection {
                host: &c.host,
                username: &c.username,
                skip_cert_verify: c.skip_cert_verify,
            })
            .collect(),
    };
    let json =
        serde_json::to_string_pretty(&on_disk).map_err(|e| format!("could not serialize config: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    restrict_permissions(path);
    Ok(())
}

/// Move cleartext passwords from an old config file into the credential store.
fn migrate(cfg: &AppConfig) -> Result<(), String> {
    for conn in cfg.connections.iter().filter(|c| !c.password.is_empty()) {
        super::secrets::store(&conn.label(), &conn.username, &conn.password)?;
    }
    Ok(())
}

/// The file no longer holds passwords, but it still names every vCenter and
/// account this operator uses. Owner-only is the right posture for that.
///
/// Unix gets a chmod. Windows gets nothing, and that is now acceptable rather
/// than a hole: the secret is in Credential Manager, not in this file.
#[cfg(unix)]
fn restrict_permissions(path: &PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &PathBuf) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_never_serializes() {
        let cfg = AppConfig {
            connections: vec![VCenterConnection::new("vc.example.com", "admin").with_password("hunter2")],
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(!json.contains("hunter2"), "password leaked into JSON: {json}");
        assert!(!json.contains("\"password\""));
    }

    #[test]
    fn password_still_deserializes_from_an_old_file() {
        let cfg: AppConfig = serde_json::from_str(
            r#"{"connections":[{"host":"vc.example.com","username":"admin","password":"hunter2"}]}"#,
        )
        .unwrap();
        assert_eq!(cfg.connections[0].password, "hunter2");
    }

    #[test]
    fn certificate_verification_is_on_when_the_file_is_silent() {
        let cfg: AppConfig =
            serde_json::from_str(r#"{"connections":[{"host":"vc.example.com","username":"admin"}]}"#).unwrap();
        assert!(
            !cfg.connections[0].skip_cert_verify,
            "a config that does not mention certificates must verify them"
        );
    }

    #[test]
    fn the_file_holds_only_host_username_and_certificate_policy() {
        let dir = std::env::temp_dir().join(format!("invar-config-test-{}", std::process::id()));
        let path = dir.join("config.json");
        let _ = std::fs::remove_dir_all(&dir);

        let mut conn = VCenterConnection::new("vc.example.com", "admin");
        conn.has_stored_password = true;
        let cfg = AppConfig { connections: vec![conn] };
        write(&path, &cfg).expect("write config");

        let text = std::fs::read_to_string(&path).expect("read back");
        assert!(text.contains("\"host\""), "{text}");
        assert!(text.contains("\"username\""), "{text}");
        assert!(text.contains("\"skip_cert_verify\""), "{text}");
        assert!(!text.contains("password"), "the file must not mention passwords: {text}");
        assert!(!text.contains("has_stored_password"), "UI-only field leaked to disk: {text}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn base_url_tolerates_a_pasted_scheme_and_trailing_slash() {
        let conn = VCenterConnection::new("https://vc.example.com/", "admin");
        assert_eq!(conn.base_url(), "https://vc.example.com");
        assert_eq!(conn.label(), "vc.example.com");
    }

    #[test]
    fn env_password_wins_and_needs_no_credential_store() {
        std::env::set_var("INVAR_PASSWORD_1", "from-env");
        let cfg = AppConfig {
            connections: vec![VCenterConnection::new("vc-env.example.com", "admin")],
        };
        let resolved = resolve(&cfg).expect("env password should resolve");
        assert_eq!(resolved[0].password, "from-env");
        std::env::remove_var("INVAR_PASSWORD_1");
    }
}
