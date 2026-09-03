//! Verify the fetchers against a live vCenter.
//!
//!   VC_HOST=vcsa91.vcrocs.local VC_USER=... VC_PASS=... cargo run --example verify [sheet]
//!
//! Prints row counts and a sample row so results can be compared against what
//! vCenter itself reports. Never prints credentials.

use sttools_lib::data::{vdisk, vhealth, vhost, vinfo, vsnapshot, Cell, Table};
use sttools_lib::vcenter::{SessionCache, VCenterConnection};

#[tokio::main]
async fn main() -> Result<(), String> {
    let conn = VCenterConnection {
        host: std::env::var("VC_HOST").map_err(|_| "set VC_HOST")?,
        username: std::env::var("VC_USER").map_err(|_| "set VC_USER")?,
        password: std::env::var("VC_PASS").map_err(|_| "set VC_PASS")?,
        skip_cert_verify: true,
    };

    let sheet = std::env::args().nth(1).unwrap_or_else(|| "vInfo".into());
    let cache = SessionCache::new();
    let conns = std::slice::from_ref(&conn);
    let table: Table = match sheet.as_str() {
        "vInfo" => vinfo::fetch_vinfo_all(conns, &cache).await,
        "vHost" => vhost::fetch_vhost_all(conns, &cache).await,
        "vDisk" => vdisk::fetch_vdisk_all(conns, &cache).await,
        "vSnapshot" => vsnapshot::fetch_vsnapshot_all(conns, &cache).await,
        "vHealth" => vhealth::fetch_vhealth_all(conns, &cache).await,
        other => return Err(format!("unknown sheet: {other}")),
    };

    for w in &table.warnings {
        println!("WARNING: {w}");
    }
    println!("sheet: {}", table.name);
    println!("columns ({}): {:?}", table.columns.len(),
        table.columns.iter().map(|c| c.label.as_str()).collect::<Vec<_>>());
    println!("rows: {}", table.rows.len());

    for row in table.rows.iter().take(3) {
        println!("---");
        for (col, cell) in table.columns.iter().zip(row) {
            let v = match cell {
                Cell::Text(s) => s.clone(),
                Cell::Number(n) => n.to_string(),
                Cell::Bool(b) => b.to_string(),
                Cell::Empty => "(empty)".into(),
            };
            println!("  {:42} {}", col.label, v.replace('\n', " / "));
        }
    }

    // Session hygiene is part of what is being verified.
    cache.close_all().await;
    Ok(())
}
