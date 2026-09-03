//! vCluster — one row per cluster.
//!
//! `summary` carries the sizing and health; `configurationEx` carries the HA
//! (`dasConfig`), DRS (`drsConfig`) and DPM (`dpmConfigInfo`) settings. Both are
//! fetched whole rather than by guessed sub-paths.
//!
//! `admissionControlPolicy` is polymorphic — its `xsi:type` says which policy is
//! in force, and the reference lab uses
//! `ClusterFailoverResourcesAdmissionControlPolicy`, whose percentages sit
//! directly on it.

use super::common::bytes_to_gib;
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

/// `ClusterComputeResource` properties this sheet reads.
pub const CLUSTER_PROPS: &[&str] =
    &["name", "summary", "configurationEx", "overallStatus", "configStatus"];

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("Name"),
        Column::text("Config status"),
        Column::text("OverallStatus"),
        Column::number("NumHosts"),
        Column::number("numEffectiveHosts"),
        Column::number("TotalCpu"),
        Column::number("NumCpuCores"),
        Column::number("NumCpuThreads"),
        Column::number("Effective Cpu"),
        Column::number("TotalMemory GiB"),
        Column::number("Num VMotions"),
        Column::bool("HA enabled"),
        Column::number("Failover Level"),
        Column::bool("AdmissionControlEnabled"),
        Column::text("Host monitoring"),
        Column::text("HB Datastore Candidate Policy"),
        Column::text("Isolation Response"),
        Column::text("Restart Priority"),
        Column::text("VM Monitoring"),
        Column::bool("DRS enabled"),
        Column::text("DRS default VM behavior"),
        Column::number("DRS vmotion rate"),
        Column::bool("DPM enabled"),
        Column::text("DPM default behavior"),
        Column::number("DPM Host Power Action Rate"),
    ]
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let mut rows = Vec::new();

    for cl in &snap.clusters {
        let Some(name) = cl.str_prop("name") else {
            return Err(format!(
                "ClusterComputeResource {} returned no name property",
                cl.moref
            ));
        };
        let summary = cl.prop("summary");
        let cfg = cl.prop("configurationEx");
        let das = cfg.and_then(|c| c.child("dasConfig"));
        let drs = cfg.and_then(|c| c.child("drsConfig"));
        let dpm = cfg.and_then(|c| c.child("dpmConfigInfo"));

        let snum = |p: &str| {
            summary.and_then(|s| s.text_at(p)).and_then(|v| v.parse::<f64>().ok())
        };
        let text = |el: Option<&crate::vcenter::xml::Element>, p: &str| {
            el.and_then(|e| e.text_at(p)).filter(|s| !s.is_empty())
        };
        let flag = |el: Option<&crate::vcenter::xml::Element>, p: &str| {
            el.and_then(|e| e.text_at(p)).map(|v| v == "true")
        };

        rows.push((
            cl.moref.clone(),
            vec![
                Cell::Text(name),
                Cell::opt_text(cl.str_prop("configStatus")),
                Cell::opt_text(cl.str_prop("overallStatus")),
                Cell::opt_num(snum("numHosts")),
                Cell::opt_num(snum("numEffectiveHosts")),
                // MHz, as vCenter reports it.
                Cell::opt_num(snum("totalCpu")),
                Cell::opt_num(snum("numCpuCores")),
                Cell::opt_num(snum("numCpuThreads")),
                Cell::opt_num(snum("effectiveCpu")),
                // totalMemory is bytes; RVTools' column is a memory size, so it
                // is converted rather than shown as a 13-digit byte count.
                Cell::opt_num(
                    summary
                        .and_then(|s| s.text_at("totalMemory"))
                        .and_then(|v| v.parse::<i64>().ok())
                        .map(bytes_to_gib),
                ),
                Cell::opt_num(snum("numVmotions")),
                Cell::opt_bool(flag(das, "enabled")),
                Cell::opt_num(
                    das.and_then(|d| d.text_at("failoverLevel"))
                        .and_then(|v| v.parse::<f64>().ok()),
                ),
                Cell::opt_bool(flag(das, "admissionControlEnabled")),
                Cell::opt_text(text(das, "hostMonitoring")),
                Cell::opt_text(text(das, "hBDatastoreCandidatePolicy")),
                Cell::opt_text(text(das, "defaultVmSettings/isolationResponse")),
                Cell::opt_text(text(das, "defaultVmSettings/restartPriority")),
                Cell::opt_text(text(das, "vmMonitoring")),
                Cell::opt_bool(flag(drs, "enabled")),
                Cell::opt_text(text(drs, "defaultVmBehavior")),
                Cell::opt_num(
                    drs.and_then(|d| d.text_at("vmotionRate")).and_then(|v| v.parse::<f64>().ok()),
                ),
                Cell::opt_bool(flag(dpm, "enabled")),
                Cell::opt_text(text(dpm, "defaultDpmBehavior")),
                Cell::opt_num(
                    dpm.and_then(|d| d.text_at("hostPowerActionRate"))
                        .and_then(|v| v.parse::<f64>().ok()),
                ),
            ],
        ));
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vCluster",
    columns,
    vm_props: &[],
    host_props: &[],
    dvs_props: &[],
    dvpg_props: &[],
    cluster_props: &[CLUSTER_PROPS],
    datastore_props: &[],
    rp_props: &[],
    wants_licenses: false,
    wants_about: false,
    source: RowSource::None,
    rows,
};

pub async fn fetch_vcluster_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_clusters, captured_snapshot, cells, col};

    fn snapshot() -> InventorySnapshot {
        captured_snapshot().with_clusters(captured_clusters())
    }

    fn at(row: &[Cell], label: &str) -> Cell {
        row[col(&columns(), label)].clone()
    }

    #[test]
    fn one_row_per_cluster() {
        let rows = cells(rows(&snapshot()).expect("named cluster"));
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn sizing_comes_off_summary() {
        let rows = cells(rows(&snapshot()).expect("named cluster"));
        let r = &rows[0];
        assert!(matches!(at(r, "NumHosts"), Cell::Number(n) if n == 3.0));
        assert!(matches!(at(r, "NumCpuCores"), Cell::Number(n) if n > 0.0));
        // totalMemory is bytes in the response and GiB in the column.
        assert!(
            matches!(at(r, "TotalMemory GiB"), Cell::Number(n) if n > 1000.0 && n < 10000.0),
            "expected GiB, got {:?}",
            at(r, "TotalMemory GiB")
        );
    }

    /// HA, DRS and DPM live in three separate blocks of `configurationEx`, not
    /// in `summary`, and each has its own `enabled` flag.
    #[test]
    fn ha_drs_and_dpm_come_off_configuration_ex() {
        let rows = cells(rows(&snapshot()).expect("named cluster"));
        let r = &rows[0];
        assert!(matches!(at(r, "HA enabled"), Cell::Bool(true)));
        assert!(matches!(at(r, "DRS enabled"), Cell::Bool(true)));
        assert!(matches!(at(r, "DPM enabled"), Cell::Bool(false)));
        assert!(matches!(at(r, "DRS default VM behavior"), Cell::Text(ref s) if s == "fullyAutomated"));
        assert!(matches!(at(r, "Isolation Response"), Cell::Text(ref s) if !s.is_empty()));
    }
}
