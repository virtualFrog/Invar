//! The post-refactor export path: one shared snapshot per vCenter feeding every
//! sheet, via `data::snapshot::fetch_tables(data::SHEETS, ..)`.
//!
//! Only compiles at 1ef59b3 and later — `snapshot` does not exist at cf626b8.
//! Comparing its output against `parity_probe`'s proves the shared snapshot
//! yields what the narrow per-sheet fetches yield.
//!
//! Usage: cargo run --example union_probe -- <out.json> <out.xlsx>

use invar_lib::data;
use invar_lib::export;
use invar_lib::vcenter::{config, SessionCache};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let out_json = args.next().unwrap_or_else(|| "union.json".into());
    let out_xlsx = args.next().unwrap_or_else(|| "union.xlsx".into());

    let dir = std::env::var("APPDATA")
        .map(|a| std::path::PathBuf::from(a).join("ch.soultec.invar"))
        .expect("APPDATA must be set");
    let cfg = config::load(&config::config_path(dir)).expect("config loads");
    assert!(!cfg.connections.is_empty(), "no vCenter configured");

    let cache = SessionCache::new();
    let servers: Vec<String> = cfg.connections.iter().map(|c| c.label()).collect();

    let started = std::time::Instant::now();
    // The same exclusion the app's export makes: sheets needing a datastore
    // file walk are not part of "fetch everything". Without this the probe
    // measures something the product never does.
    let sheets: Vec<&data::snapshot::SheetSpec> =
        data::SHEETS.iter().copied().filter(|s| !s.wants_files).collect();
    let tables = data::snapshot::fetch_tables(&sheets, &cfg.connections, &cache).await;
    let elapsed = started.elapsed();

    for t in &tables {
        eprintln!(
            "{:<10} rows={:<6} cols={:<4} warnings={:?}",
            t.name, t.rows.len(), t.columns.len(), t.warnings
        );
    }
    eprintln!("shared-snapshot fetch took {:.1}s", elapsed.as_secs_f64());

    std::fs::write(&out_json, serde_json::to_string_pretty(&tables).expect("serializes"))
        .expect("writes json");
    export::write_workbook(&tables, &servers, std::path::Path::new(&out_xlsx))
        .expect("writes xlsx");
    eprintln!("wrote {out_json} and {out_xlsx}");

    cache.close_all().await;
}
