//! Where vCenter passwords live.
//!
//! Not in `config.json`. That file used to hold them as plain strings, chmod'd
//! to `0600` on Unix and left with default ACLs on Windows, where the
//! `#[cfg(not(unix))]` arm of `restrict_permissions` was an empty function —
//! so on a first-class target every local administrator could read a vSphere
//! admin password out of a JSON file.
//!
//! Passwords now go to the OS credential store: Keychain on macOS, Credential
//! Manager on Windows, Secret Service on *nix.
//!
//! ## The headless case
//!
//! A Linux service has no Secret Service: no D-Bus session, no unlocked
//! keyring. `keyring::Entry::new` reports that as `NoDefaultStore` rather than
//! failing obscurely, and `invar-export` is expected to take its password from
//! the environment instead. `config::resolve` checks the environment first for
//! exactly that reason, so a container or a systemd unit never depends on a
//! desktop credential store being present.

/// Credential-store service name. Shared with the bundle identifier so a user
/// looking through Keychain Access finds entries under a name they recognise.
pub const SERVICE: &str = "ch.soultec.invar";

/// The credential-store account for one connection.
///
/// Host *and* user, matching `VCenterConnection::cache_key`: two accounts on
/// one vCenter are two credentials, not one that overwrites the other.
pub fn account(host_label: &str, username: &str) -> String {
    format!("{host_label}|{username}")
}

/// Whether a credential store could be opened at all.
///
/// False on a headless Linux box with no Secret Service. Callers use this to
/// explain the absence rather than reporting every lookup as a failure.
pub fn available() -> bool {
    keyring::Entry::store_status().is_ok()
}

/// The stored password for a connection, or `None` when none is stored.
///
/// A missing entry is `Ok(None)`, not an error: a connection that has never
/// been given a password is an ordinary state, not a fault.
pub fn load(host_label: &str, username: &str) -> Result<Option<String>, String> {
    let entry = match keyring::Entry::new(SERVICE, &account(host_label, username)) {
        Ok(e) => e,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(e) => return Err(describe(host_label, e)),
    };
    match entry.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(describe(host_label, e)),
    }
}

/// Store a password, replacing any previous one for the same host and user.
pub fn store(host_label: &str, username: &str, password: &str) -> Result<(), String> {
    keyring::Entry::new(SERVICE, &account(host_label, username))
        .and_then(|e| e.set_password(password))
        .map_err(|e| describe(host_label, e))
}

/// Forget a password. A credential that is already gone is not an error.
pub fn forget(host_label: &str, username: &str) -> Result<(), String> {
    match keyring::Entry::new(SERVICE, &account(host_label, username)) {
        Ok(e) => match e.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(describe(host_label, e)),
        },
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(describe(host_label, e)),
    }
}

/// Turn a keyring error into something an administrator can act on.
///
/// `NoDefaultStore` is the one worth spelling out: it is not a broken
/// installation, it is a machine with no credential store, and the fix is the
/// environment variable rather than anything inside the app.
fn describe(host_label: &str, e: keyring::Error) -> String {
    match e {
        keyring::Error::NoDefaultStore => format!(
            "no OS credential store is available for {host_label}. \
             On a headless machine, pass the password in INVAR_PASSWORD_<n> instead."
        ),
        other => format!("credential store failed for {host_label}: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_separates_users_on_one_host() {
        assert_ne!(
            account("vc.example.com", "alice@vsphere.local"),
            account("vc.example.com", "bob@vsphere.local")
        );
    }

    /// A real round trip through the platform credential store.
    ///
    /// Ignored by default: it writes to the developer's Keychain (or Credential
    /// Manager, or Secret Service), which is not something a plain `cargo test`
    /// should do, and CI runners have no unlocked store to write to. Run it by
    /// hand after touching this module, because a keyring API mismatch compiles
    /// perfectly and only fails at the moment a user saves a password:
    ///
    /// ```text
    /// cargo test --lib secrets -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "writes to the real OS credential store"]
    fn round_trips_through_the_real_credential_store() {
        assert!(available(), "no credential store on this machine: {:?}", keyring::Entry::store_status());

        let host = "invar-selftest.example.invalid";
        let user = "selftest@vsphere.local";

        store(host, user, "first-secret").expect("store");
        assert_eq!(load(host, user).expect("load"), Some("first-secret".to_string()));

        // Storing again must replace, not duplicate.
        store(host, user, "second-secret").expect("re-store");
        assert_eq!(load(host, user).expect("load again"), Some("second-secret".to_string()));

        forget(host, user).expect("forget");
        assert_eq!(load(host, user).expect("load after forget"), None);

        // Forgetting something already gone is not an error.
        forget(host, user).expect("idempotent forget");
    }

    #[test]
    fn account_separates_hosts_for_one_user() {
        assert_ne!(
            account("vc-a.example.com", "administrator@vsphere.local"),
            account("vc-b.example.com", "administrator@vsphere.local")
        );
    }
}
