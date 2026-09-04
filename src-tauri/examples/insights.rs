//! Print the Environment Overview rollup from a live vCenter.
//!
//!   VC_HOST=… VC_USER=… VC_PASS=… cargo run --example insights

use invar_lib::data::insights;
use invar_lib::vcenter::{SessionCache, VCenterConnection};

#[tokio::main]
async fn main() -> Result<(), String> {
    let conn = VCenterConnection {
        host: std::env::var("VC_HOST").map_err(|_| "set VC_HOST")?,
        username: std::env::var("VC_USER").map_err(|_| "set VC_USER")?,
        password: std::env::var("VC_PASS").map_err(|_| "set VC_PASS")?,
        skip_cert_verify: true,
    };
    let cache = SessionCache::new();
    let i = insights::fetch_insights_all(std::slice::from_ref(&conn), &cache).await;

    for w in &i.warnings {
        println!("WARNING: {w}");
    }
    println!("servers          {:?}", i.servers);
    println!("datacenters      {}", i.datacenters);
    println!("clusters         {}", i.clusters);
    println!("hosts            {} (maint {}, disconnected {})", i.hosts, i.hosts_in_maintenance, i.hosts_disconnected);
    println!("TOTAL CORES      {}", i.cores);
    println!("DRAM GiB         {}", i.dram_gib);
    println!("memory total GiB {}", i.memory_total_gib);
    println!("VMs              {} ({} on)", i.vms_total, i.vms_powered_on);
    println!("vCPUs            {}  ({:?} per core)", i.vcpus, i.vcpu_core_ratio);
    println!("vRAM GiB         {}", i.vram_gib);
    println!("datastores       {}", i.datastores);
    println!("TOTAL STORAGE    {} GiB ({:.2} TiB)", i.storage_capacity_gib, i.storage_capacity_gib / 1024.0);
    println!("  used           {} GiB ({}%)", i.storage_used_gib, i.storage_used_percent);
    println!("  free           {} GiB", i.storage_free_gib);
    for t in &i.storage_by_type {
        println!("  {:6} {:>2} stores {:>10.1} GiB cap {:>10.1} GiB used", t.kind, t.datastores, t.capacity_gib, t.used_gib);
    }
    for c in &i.cluster_summaries {
        println!("  cluster {:8} hosts={} cores={} dram={} GiB", c.name, c.hosts, c.cores, c.dram_gib);
    }
    for d in &i.top_datastores {
        println!("  {:16} {:>5.1}% used", d.name, d.used_percent);
    }
    // `--json` prints the exact payload the UI receives.
    if std::env::args().any(|a| a == "--json") {
        println!("{}", serde_json::to_string_pretty(&i).map_err(|e| e.to_string())?);
    }

    cache.close_all().await;
    Ok(())
}
