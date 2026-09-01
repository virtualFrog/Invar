//! Connection settings.
//!
//! The config holds a *list* of vCenters from day one — every fetch path is an
//! aggregation over connections, so single-server is just the one-element case.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct VCenterConnection {
    /// Hostname or IP, no scheme. `https://` is assumed.
    pub host: String,
    pub username: String,
    pub password: String,
    /// Lab vCenters use self-signed certificates.
    #[serde(default = "default_true")]
    pub skip_cert_verify: bool,
}

fn default_true() -> bool {
    true
}

impl VCenterConnection {
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

pub fn load(path: &PathBuf) -> Result<AppConfig, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("config.json is not valid: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AppConfig::default()),
        Err(e) => Err(format!("could not read {}: {e}", path.display())),
    }
}

pub fn save(path: &PathBuf, cfg: &AppConfig) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(cfg).map_err(|e| format!("could not serialize config: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    restrict_permissions(path);
    Ok(())
}

/// The file holds vCenter passwords, so keep it owner-only.
#[cfg(unix)]
fn restrict_permissions(path: &PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &PathBuf) {}
