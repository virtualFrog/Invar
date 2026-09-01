//! Generate the HTML topology report from a live vCenter.
//!
//!   VC_HOST=… VC_USER=… VC_PASS=… cargo run --example report -- out.html

use vcenter_inventory_lib::data::topology;
use vcenter_inventory_lib::report;
use vcenter_inventory_lib::vcenter::{SessionCache, VCenterConnection};

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
        .unwrap_or_else(|| std::env::temp_dir().join(report::default_filename()).display().to_string());

    let cache = SessionCache::new();
    let t = topology::fetch_topology_all(std::slice::from_ref(&conn), &cache).await;
    for w in &t.warnings {
        println!("WARNING: {w}");
    }
    for s in &t.servers {
        println!(
            "{}: {} datacenters, {} clusters, {} hosts, {} datastores",
            s.server,
            s.datacenters.len(),
            s.clusters.len(),
            s.all_hosts().len(),
            s.datastores.len()
        );
        for (name, hosts) in &s.clusters {
            println!("   cluster {name}: {:?}", hosts.iter().map(|h| &h.name).collect::<Vec<_>>());
        }
        for d in &s.datastores {
            println!(
                "   {:16} {:5} {:>9.1} GiB  {:>5.1}% used  {} mounts  {} VMs",
                d.name,
                d.kind.clone().unwrap_or_default(),
                d.capacity_gib.unwrap_or(0.0),
                d.used_percent().unwrap_or(0.0),
                d.mounted_by.len(),
                d.vm_count
            );
        }
    }
    std::fs::write(&out, report::render(&t)).map_err(|e| e.to_string())?;
    println!("wrote {out}");
    cache.close_all().await;
    Ok(())
}
