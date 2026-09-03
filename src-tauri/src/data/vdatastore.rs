//! vDatastore — one row per datastore.
//!
//! `summary` carries capacity, free space, type and accessibility. `host` and
//! `vm` are moref arrays, so the mount count and VM count are lengths rather
//! than extra queries.
//!
//! Provisioned space is derived: vCenter reports capacity, free space and
//! `uncommitted` (space thin disks could still claim), so provisioned is
//! capacity − free + uncommitted. That is the figure RVTools shows, and it can
//! exceed capacity on a thin-provisioned datastore — which is the point of the
//! column.

use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

/// `Datastore` properties this sheet reads.
pub const DATASTORE_PROPS: &[&str] =
    &["name", "summary", "host", "vm", "iormConfiguration", "overallStatus"];

const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;

fn to_mib(bytes: i64) -> f64 {
    (bytes as f64 / BYTES_PER_MIB * 100.0).round() / 100.0
}

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("Name"),
        Column::text("Config status"),
        Column::text("Type"),
        Column::bool("Accessible"),
        Column::number("# VMs"),
        Column::number("# Hosts"),
        Column::number("Capacity MiB"),
        Column::number("Provisioned MiB"),
        Column::number("In Use MiB"),
        Column::number("Free MiB"),
        Column::number("Free %"),
        Column::bool("SIOC enabled"),
        Column::number("SIOC Threshold"),
        Column::bool("MHA"),
        Column::text("URL"),
    ]
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let mut rows = Vec::new();

    for ds in &snap.datastores {
        let Some(name) = ds.str_prop("name") else {
            return Err(format!("Datastore {} returned no name property", ds.moref));
        };
        let summary = ds.prop("summary");
        let iorm = ds.prop("iormConfiguration");
        let n = |p: &str| {
            summary.and_then(|s| s.text_at(p)).and_then(|v| v.parse::<i64>().ok())
        };

        let capacity = n("capacity");
        let free = n("freeSpace");
        let uncommitted = n("uncommitted");
        let in_use = match (capacity, free) {
            (Some(c), Some(f)) => Some(c - f),
            _ => None,
        };
        // Thin disks can claim more than is currently written, so provisioned
        // is what is used plus what is still promised.
        let provisioned = match (in_use, uncommitted) {
            (Some(u), Some(unc)) => Some(u + unc),
            (Some(u), None) => Some(u),
            _ => None,
        };
        let free_pct = match (capacity, free) {
            (Some(c), Some(f)) if c > 0 => Some((f as f64 / c as f64 * 10000.0).round() / 100.0),
            _ => None,
        };

        rows.push((
            ds.moref.clone(),
            vec![
                Cell::Text(name),
                Cell::opt_text(ds.str_prop("overallStatus")),
                Cell::opt_text(summary.and_then(|s| s.text_at("type")).filter(|s| !s.is_empty())),
                Cell::opt_bool(summary.and_then(|s| s.text_at("accessible")).map(|v| v == "true")),
                // Moref arrays: the counts are lengths, not extra queries.
                Cell::Number(ds.array_prop("vm").len() as f64),
                Cell::Number(ds.array_prop("host").len() as f64),
                Cell::opt_num(capacity.map(to_mib)),
                Cell::opt_num(provisioned.map(to_mib)),
                Cell::opt_num(in_use.map(to_mib)),
                Cell::opt_num(free.map(to_mib)),
                Cell::opt_num(free_pct),
                Cell::opt_bool(iorm.and_then(|i| i.text_at("enabled")).map(|v| v == "true")),
                Cell::opt_num(
                    iorm.and_then(|i| i.text_at("congestionThreshold"))
                        .and_then(|v| v.parse::<f64>().ok()),
                ),
                // Multiple-host access: whether more than one host can reach it.
                Cell::opt_bool(
                    summary.and_then(|s| s.text_at("multipleHostAccess")).map(|v| v == "true"),
                ),
                Cell::opt_text(summary.and_then(|s| s.text_at("url")).filter(|s| !s.is_empty())),
            ],
        ));
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vDatastore",
    columns,
    vm_props: &[],
    host_props: &[],
    dvs_props: &[],
    dvpg_props: &[],
    cluster_props: &[],
    datastore_props: &[DATASTORE_PROPS],
    rp_props: &[],
    wants_licenses: false,
    wants_about: false,
    source: RowSource::None,
    rows,
};

pub async fn fetch_vdatastore_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_datastores, captured_snapshot, cells, col};

    fn snapshot() -> InventorySnapshot {
        captured_snapshot().with_datastores(captured_datastores())
    }

    fn at(row: &[Cell], label: &str) -> Cell {
        row[col(&columns(), label)].clone()
    }

    #[test]
    fn one_row_per_datastore() {
        let snap = snapshot();
        let rows = cells(rows(&snap).expect("named datastore"));
        assert_eq!(rows.len(), snap.datastores.len());
        assert!(!rows.is_empty());
    }

    /// Capacity and free are bytes in the response and MiB in the columns, and
    /// in-use is derived from the two rather than reported.
    #[test]
    fn sizes_are_mib_and_in_use_is_derived() {
        let rows = cells(rows(&snapshot()).expect("named datastore"));
        for r in &rows {
            if let (Cell::Number(c), Cell::Number(u), Cell::Number(f)) =
                (at(r, "Capacity MiB"), at(r, "In Use MiB"), at(r, "Free MiB"))
            {
                assert!((c - (u + f)).abs() < 1.0, "capacity {c} = used {u} + free {f}");
            }
        }
    }

    /// `host` and `vm` are moref arrays, so the counts are lengths. The vSAN
    /// datastore is mounted by every host; the local VMFS ones by exactly one.
    #[test]
    fn mount_and_vm_counts_are_array_lengths() {
        let rows = cells(rows(&snapshot()).expect("named datastore"));
        assert!(
            rows.iter().any(|r| matches!(at(r, "# Hosts"), Cell::Number(n) if n > 1.0)),
            "a shared datastore should be mounted by more than one host"
        );
        assert!(rows.iter().any(|r| matches!(at(r, "# VMs"), Cell::Number(n) if n > 0.0)));
    }
}
