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
}

impl SessionCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// A live session for this connection, reused when still fresh.
    pub async fn get(&self, conn: &VCenterConnection) -> Result<Arc<Session>, String> {
        let key = conn.cache_key();

        let stale = {
            let mut entries = self.entries.lock().await;
            match entries.get(&key) {
                Some(s) if s.is_fresh() => return Ok(Arc::clone(s)),
                Some(_) => entries.remove(&key),
                None => None,
            }
        };
        // Retire the expired session outside the lock so one slow vCenter does
        // not block lookups for the others.
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

    /// Log out of everything. Called on shutdown, for SIGINT and SIGTERM alike.
    pub async fn close_all(&self) {
        let sessions: Vec<Arc<Session>> = self.entries.lock().await.drain().map(|(_, s)| s).collect();
        for s in sessions {
            s.logout().await;
        }
    }
}
