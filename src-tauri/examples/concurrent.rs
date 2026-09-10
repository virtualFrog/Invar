//! Prove the session cache logs in once under concurrent fetches.
//!
//!   VC_HOST=… VC_USER=… VC_PASS=… cargo run --example concurrent
//!
//! Fires every sheet at one vCenter simultaneously; the cache must produce a
//! single session, not one per sheet.

use std::sync::Arc;
use invar_lib::data::{vdisk, vhost, vinfo, vsnapshot};
use invar_lib::vcenter::{SessionCache, VCenterConnection};

#[tokio::main]
async fn main() -> Result<(), String> {
    let mut conn = VCenterConnection::new(
        &std::env::var("VC_HOST").map_err(|_| "set VC_HOST")?,
        &std::env::var("VC_USER").map_err(|_| "set VC_USER")?,
    )
    .with_password(&std::env::var("VC_PASS").map_err(|_| "set VC_PASS")?);
    // These examples point at lab vCenters with self-signed certificates.
    // Set VC_VERIFY_CERT=1 to run them against one with a trusted chain.
    if std::env::var("VC_VERIFY_CERT").as_deref() != Ok("1") {
        conn = conn.trusting_invalid_certs();
    }
    let cache = Arc::new(SessionCache::new());
    let conns = vec![conn];

    let (a, b, c, d) = tokio::join!(
        vinfo::fetch_vinfo_all(&conns, &cache),
        vhost::fetch_vhost_all(&conns, &cache),
        vdisk::fetch_vdisk_all(&conns, &cache),
        vsnapshot::fetch_vsnapshot_all(&conns, &cache),
    );
    for t in [&a, &b, &c, &d] {
        println!("{:10} {:>4} rows  warnings: {:?}", t.name, t.rows.len(), t.warnings);
    }

    cache.close_all().await;
    Ok(())
}
