//! vRP — one row per resource pool.
//!
//! `config` carries the CPU and memory allocations as configured; `summary`
//! carries the runtime figures and quick stats. Both are fetched whole.
//!
//! Every cluster has a root pool named `Resources` whether or not anyone made
//! one, and vSphere Supervisor creates one per namespace, which is why the
//! reference lab has 43 of them.

use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

/// `ResourcePool` properties this sheet reads.
pub const RP_PROPS: &[&str] = &["name", "config", "summary", "owner", "parent", "vm"];

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("Resource Pool name"),
        Column::text("Status"),
        Column::number("# VMs"),
        Column::number("CPU limit"),
        Column::number("CPU reservation"),
        Column::text("CPU level"),
        Column::number("CPU shares"),
        Column::bool("CPU expandableReservation"),
        Column::number("Mem limit"),
        Column::number("Mem reservation"),
        Column::text("Mem level"),
        Column::number("Mem shares"),
        Column::bool("Mem expandableReservation"),
        Column::number("QS overallCpuUsage"),
        Column::number("QS overallCpuDemand"),
        Column::number("QS staticCpuEntitlement"),
        Column::number("QS distributedCpuEntitlement"),
        Column::number("QS guestMemoryUsage"),
        Column::number("QS hostMemoryUsage"),
        Column::number("QS privateMemory"),
        Column::number("QS sharedMemory"),
        Column::number("QS swappedMemory"),
        Column::number("QS balloonedMemory"),
        Column::number("QS overheadMemory"),
        Column::number("QS consumedOverheadMemory"),
        Column::number("QS compressedMemory"),
        Column::number("QS staticMemoryEntitlement"),
        Column::number("QS distributedMemoryEntitlement"),
    ]
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let mut rows = Vec::new();

    for rp in &snap.resource_pools {
        let Some(name) = rp.str_prop("name") else {
            return Err(format!("ResourcePool {} returned no name property", rp.moref));
        };
        let config = rp.prop("config");
        let summary = rp.prop("summary");
        let qs = summary.and_then(|s| s.child("quickStats"));

        let cnum = |p: &str| {
            config.and_then(|c| c.text_at(p)).and_then(|v| v.parse::<f64>().ok())
        };
        let cflag = |p: &str| config.and_then(|c| c.text_at(p)).map(|v| v == "true");
        let ctext = |p: &str| config.and_then(|c| c.text_at(p)).filter(|s| !s.is_empty());
        let q = |p: &str| {
            Cell::opt_num(qs.and_then(|s| s.text_at(p)).and_then(|v| v.parse::<f64>().ok()))
        };

        rows.push((
            rp.moref.clone(),
            vec![
                Cell::Text(name),
                Cell::opt_text(
                    summary.and_then(|s| s.text_at("runtime/overallStatus")).filter(|s| !s.is_empty()),
                ),
                Cell::Number(rp.array_prop("vm").len() as f64),
                // -1 is vCenter's "unlimited", shown as-is.
                Cell::opt_num(cnum("cpuAllocation/limit")),
                Cell::opt_num(cnum("cpuAllocation/reservation")),
                Cell::opt_text(ctext("cpuAllocation/shares/level")),
                Cell::opt_num(cnum("cpuAllocation/shares/shares")),
                Cell::opt_bool(cflag("cpuAllocation/expandableReservation")),
                Cell::opt_num(cnum("memoryAllocation/limit")),
                Cell::opt_num(cnum("memoryAllocation/reservation")),
                Cell::opt_text(ctext("memoryAllocation/shares/level")),
                Cell::opt_num(cnum("memoryAllocation/shares/shares")),
                Cell::opt_bool(cflag("memoryAllocation/expandableReservation")),
                q("overallCpuUsage"),
                q("overallCpuDemand"),
                q("staticCpuEntitlement"),
                q("distributedCpuEntitlement"),
                q("guestMemoryUsage"),
                q("hostMemoryUsage"),
                q("privateMemory"),
                q("sharedMemory"),
                q("swappedMemory"),
                q("balloonedMemory"),
                q("overheadMemory"),
                q("consumedOverheadMemory"),
                q("compressedMemory"),
                q("staticMemoryEntitlement"),
                q("distributedMemoryEntitlement"),
            ],
        ));
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vRP",
    columns,
    vm_props: &[],
    host_props: &[],
    dvs_props: &[],
    dvpg_props: &[],
    cluster_props: &[],
    datastore_props: &[],
    rp_props: &[RP_PROPS],
    wants_licenses: false,
    wants_about: false,
    wants_files: false,
    source: RowSource::None,
    rows,
};

pub async fn fetch_vrp_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_resource_pools, captured_snapshot, cells, col};

    fn snapshot() -> InventorySnapshot {
        captured_snapshot().with_resource_pools(captured_resource_pools())
    }

    fn at(row: &[Cell], label: &str) -> Cell {
        row[col(&columns(), label)].clone()
    }

    #[test]
    fn one_row_per_resource_pool() {
        let snap = snapshot();
        let rows = cells(rows(&snap).expect("named pool"));
        assert_eq!(rows.len(), snap.resource_pools.len());
        assert!(!rows.is_empty());
    }

    /// Allocations come from `config`, quick stats from `summary/quickStats`.
    /// They are different objects and reading the wrong one gives blanks.
    #[test]
    fn allocations_and_quick_stats_come_from_different_objects() {
        let rows = cells(rows(&snapshot()).expect("named pool"));
        let r = &rows[0];
        assert!(matches!(at(r, "CPU level"), Cell::Text(_)));
        assert!(matches!(at(r, "CPU reservation"), Cell::Number(_)));
        assert!(matches!(at(r, "QS overallCpuUsage"), Cell::Number(_)));
        assert!(matches!(at(r, "QS privateMemory"), Cell::Number(_)));
    }

    /// Every cluster has a root pool called `Resources` whether or not anyone
    /// created one, so it must appear rather than be filtered as noise.
    #[test]
    fn the_root_pool_is_a_row() {
        let rows = cells(rows(&snapshot()).expect("named pool"));
        assert!(
            rows.iter().any(
                |r| matches!(at(r, "Resource Pool name"), Cell::Text(ref s) if s == "Resources")
            ),
            "the cluster root pool should be present"
        );
    }
}
