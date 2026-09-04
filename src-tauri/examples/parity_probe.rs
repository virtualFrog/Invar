//! Live parity probe: fetch every sheet from the configured vCenter(s), dump the
//! full table as deterministic JSON, and write a real xlsx export.
//!
//! Deliberately written against only `fetch_<sheet>_all` and `write_workbook`,
//! whose signatures are byte-identical at cf626b8 and 1ef59b3, so the same file
//! compiles and runs at both commits and the two dumps can be diffed directly.
//!
//! The five separate calls are also exactly what `fetch_all_tables` did at
//! cf626b8, so this path *is* the pre-refactor export path.
//!
//! Usage: cargo run --example parity_probe -- <out.json> <out.xlsx>

use invar_lib::data::{vdisk, vhealth, vhost, vinfo, vsnapshot};
use invar_lib::export;
use invar_lib::vcenter::{config, SessionCache};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let out_json = args.next().unwrap_or_else(|| "probe.json".into());
    let out_xlsx = args.next().unwrap_or_else(|| "probe.xlsx".into());

    let dir = std::env::var("APPDATA")
        .map(|a| std::path::PathBuf::from(a).join("ch.soultec.invar"))
        .expect("APPDATA must be set");
    let path = config::config_path(dir);
    let cfg = config::load(&path).expect("config loads");
    eprintln!("config: {} ({} connection(s))", path.display(), cfg.connections.len());
    assert!(!cfg.connections.is_empty(), "no vCenter configured");

    let cache = SessionCache::new();
    let servers: Vec<String> = cfg.connections.iter().map(|c| c.label()).collect();

    let started = std::time::Instant::now();
    let tables = vec![
        vinfo::fetch_vinfo_all(&cfg.connections, &cache).await,
        vhost::fetch_vhost_all(&cfg.connections, &cache).await,
        vdisk::fetch_vdisk_all(&cfg.connections, &cache).await,
        vsnapshot::fetch_vsnapshot_all(&cfg.connections, &cache).await,
        vhealth::fetch_vhealth_all(&cfg.connections, &cache).await,
    ];
    let elapsed = started.elapsed();

    for t in &tables {
        eprintln!(
            "{:<10} rows={:<6} cols={:<4} warnings={:?}",
            t.name, t.rows.len(), t.columns.len(), t.warnings
        );
    }
    eprintln!("per-sheet fetch took {:.1}s", elapsed.as_secs_f64());

    std::fs::write(&out_json, serde_json::to_string_pretty(&tables).expect("serializes"))
        .expect("writes json");
    export::write_workbook(&tables, &servers, std::path::Path::new(&out_xlsx))
        .expect("writes xlsx");
    eprintln!("wrote {out_json} and {out_xlsx}");

    cache.close_all().await;
}
