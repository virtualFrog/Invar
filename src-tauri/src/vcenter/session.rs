//! Session cache.
//!
//! vCenter sessions linger until a ~30 minute idle timeout, so logging in per
//! call leaks them by the hundred. Sessions are cached per host+user and
//! refreshed on a TTL comfortably inside that timeout.

use super::config::VCenterConnection;
use super::rest::RestClient;
use super::soap::SoapClient;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Safely inside vCenter's ~30 minute idle timeout.
const TTL: Duration = Duration::from_secs(15 * 60);

pub struct Session {
    pub rest: RestClient,
    pub soap: SoapClient,
    established: Instant,
}

impl Session {
    fn is_fresh(&self) -> bool {
        self.established.elapsed() < TTL
    }

    async fn logout(&self) {
        if let Err(e) = self.rest.logout().await {
            eprintln!("warning: REST logout failed: {e}");
        }
        if let Err(e) = self.soap.logout().await {
            eprintln!("warning: SOAP logout failed: {e}");
        }
    }
}

#[derive(Default)]
pub struct SessionCache {
    entries: Mutex<HashMap<String, Arc<Session>>>,
    /// Per-key login guard. Logging in cannot hold the `entries` lock — a slow
    /// or unreachable vCenter would block every other server's lookups — so
    /// concurrent callers for the *same* key would otherwise each log in and
    /// all but one session would be orphaned. Serializing per key means one
    /// login per vCenter no matter how many sheets are fetched at once.
    logins: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl SessionCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// A live session for this connection, reused when still fresh.
    pub async fn get(&self, conn: &VCenterConnection) -> Result<Arc<Session>, String> {
        let key = conn.cache_key();

        if let Some(session) = self.fresh(&key).await {
            return Ok(session);
        }

        let guard = {
            let mut logins = self.logins.lock().await;
            Arc::clone(logins.entry(key.clone()).or_default())
        };
        let _login = guard.lock().await;

        // Another caller may have logged in while this one waited for the guard.
        if let Some(session) = self.fresh(&key).await {
            return Ok(session);
        }

        let stale = self.entries.lock().await.remove(&key);
        // Retire the expired session outside the entries lock so one slow
        // vCenter does not block lookups for the others.
        if let Some(old) = stale {
            old.logout().await;
        }

        let mut rest = RestClient::new(conn)?;
        rest.login(conn).await?;
        let mut soap = SoapClient::new(conn)?;
        if let Err(e) = soap.login(conn).await {
            let _ = rest.logout().await;
            return Err(e);
        }

        let session = Arc::new(Session { rest, soap, established: Instant::now() });
        self.entries.lock().await.insert(key, Arc::clone(&session));
        Ok(session)
    }

    /// The cached session for this key, if it is still within its TTL.
    async fn fresh(&self, key: &str) -> Option<Arc<Session>> {
        let entries = self.entries.lock().await;
        entries.get(key).filter(|s| s.is_fresh()).map(Arc::clone)
    }

    /// Log out of everything. Called on shutdown, for SIGINT and SIGTERM alike.
    pub async fn close_all(&self) {
        let sessions: Vec<Arc<Session>> = self.entries.lock().await.drain().map(|(_, s)| s).collect();
        for s in sessions {
            s.logout().await;
        }
    }
}
