//! vCenter REST client.
//!
//! Covers the cheap list endpoints. Both namespaces (`/rest/vcenter/*` and
//! `/api/vcenter/*`) share one session token.

use super::config::VCenterConnection;
use serde_json::Value;

pub struct RestClient {
    http: reqwest::Client,
    base: String,
    pub token: Option<String>,
}

impl RestClient {
    pub fn new(conn: &VCenterConnection) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(conn.skip_cert_verify)
            .build()
            .map_err(|e| format!("could not build HTTP client: {e}"))?;
        Ok(Self { http, base: conn.base_url(), token: None })
    }

    pub async fn login(&mut self, conn: &VCenterConnection) -> Result<(), String> {
        let resp = self
            .http
            .post(format!("{}/rest/com/vmware/cis/session", self.base))
            .basic_auth(&conn.username, Some(&conn.password))
            .send()
            .await
            .map_err(|e| format!("REST login failed: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            // Deliberately does not echo the response body — it can repeat back
            // request details, and a 401 needs no explanation beyond itself.
            return Err(format!("REST login rejected by {} (HTTP {status})", self.base));
        }
        let body: Value = resp.json().await.map_err(|e| format!("REST login returned non-JSON: {e}"))?;
        let token = body
            .get("value")
            .and_then(Value::as_str)
            .ok_or("REST login response had no session token")?;
        self.token = Some(token.to_string());
        Ok(())
    }

    pub async fn logout(&self) -> Result<(), String> {
        let Some(token) = &self.token else { return Ok(()) };
        self.http
            .delete(format!("{}/rest/com/vmware/cis/session", self.base))
            .header("vmware-api-session-id", token)
            .send()
            .await
            .map_err(|e| format!("REST logout failed: {e}"))?;
        Ok(())
    }

    /// GET a path (e.g. `/rest/vcenter/host`) and return the decoded JSON.
    pub async fn get(&self, path: &str) -> Result<Value, String> {
        let token = self.token.as_ref().ok_or("REST client is not logged in")?;
        let resp = self
            .http
            .get(format!("{}{path}", self.base))
            .header("vmware-api-session-id", token)
            .send()
            .await
            .map_err(|e| format!("GET {path} failed: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("GET {path} returned HTTP {status}"));
        }
        resp.json().await.map_err(|e| format!("GET {path} returned non-JSON: {e}"))
    }

    /// GET a legacy `/rest/*` endpoint and unwrap its `value` envelope.
    pub async fn get_value(&self, path: &str) -> Result<Value, String> {
        let body = self.get(path).await?;
        Ok(body.get("value").cloned().unwrap_or(body))
    }
}
