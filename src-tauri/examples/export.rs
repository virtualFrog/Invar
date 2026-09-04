//! Generate an .xlsx export from a live vCenter, for format verification.
//!
//!   VC_HOST=… VC_USER=… VC_PASS=… cargo run --example export -- /path/out.xlsx

use invar_lib::data::{vdisk, vhealth, vhost, vinfo, vsnapshot};
use invar_lib::export;
use invar_lib::vcenter::{SessionCache, VCenterConnection};

#[tokio::main]
async fn main() -> Result<(), String> {
    let conn = VCenterConnection {
        host: std::env::var("VC_HOST").map_err(|_| "set VC_HOST")?,
        username: std::env::var("VC_USER").map_err(|_| "set VC_USER")?,
        password: std::env::var("VC_PASS").map_err(|_| "set VC_PASS")?,
        skip_cert_verify: true,
    };
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| std::env::temp_dir().join(export::default_filename()).display().to_string());

    let cache = SessionCache::new();
    let conns = std::slice::from_ref(&conn);
    let tables = vec![
        vinfo::fetch_vinfo_all(conns, &cache).await,
        vhost::fetch_vhost_all(conns, &cache).await,
        vdisk::fetch_vdisk_all(conns, &cache).await,
        vsnapshot::fetch_vsnapshot_all(conns, &cache).await,
        vhealth::fetch_vhealth_all(conns, &cache).await,
    ];
    for t in &tables {
        println!("{:10} {:>4} rows  warnings: {:?}", t.name, t.rows.len(), t.warnings);
    }
    let servers = vec![conn.label()];
    export::write_workbook(&tables, &servers, std::path::Path::new(&out))?;
    println!("wrote {out}");

    cache.close_all().await;
    Ok(())
}
