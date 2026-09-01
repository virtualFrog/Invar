//! Prove the session cache logs in once under concurrent fetches.
//!
//!   VC_HOST=… VC_USER=… VC_PASS=… cargo run --example concurrent
//!
//! Fires every sheet at one vCenter simultaneously; the cache must produce a
//! single session, not one per sheet.

use std::sync::Arc;
use vcenter_inventory_lib::data::{vdisk, vhost, vinfo, vsnapshot};
use vcenter_inventory_lib::vcenter::{SessionCache, VCenterConnection};

#[tokio::main]
async fn main() -> Result<(), String> {
    let conn = VCenterConnection {
        host: std::env::var("VC_HOST").map_err(|_| "set VC_HOST")?,
        username: std::env::var("VC_USER").map_err(|_| "set VC_USER")?,
        password: std::env::var("VC_PASS").map_err(|_| "set VC_PASS")?,
        skip_cert_verify: true,
    };
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
