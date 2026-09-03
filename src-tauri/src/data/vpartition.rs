//! vPartition — one row per guest filesystem.
//!
//! Reads `guest.disk` from the shared VM snapshot, so it adds no inventory
//! walk. This is guest-reported, not host-reported: it is what the operating
//! system says about its mounted filesystems, which is why it differs from
//! vDisk and why a VM without VMware Tools contributes no rows at all.
//!
//! `guest.disk` is a `GuestDiskInfo[]`, so its elements carry the type name
//! (`<GuestDiskInfo>`), not the field name — verified live. Reading the field
//! name would yield zero rows and no error.

use super::common::{VmContext, VM_CONTEXT_PROPS};
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

/// `VirtualMachine` properties this sheet reads.
pub const VM_PROPS: &[&str] = &["guest.disk"];

const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;

fn to_mib(bytes: i64) -> f64 {
    (bytes as f64 / BYTES_PER_MIB * 100.0).round() / 100.0
}

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("VM"),
        Column::text("Powerstate"),
        Column::bool("Template"),
        Column::text("Disk"),
        Column::text("Filesystem"),
        Column::number("Capacity MiB"),
        Column::number("Consumed MiB"),
        Column::number("Free MiB"),
        Column::number("Free %"),
        Column::text("Host"),
        Column::text("Annotation"),
    ]
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let hosts = &snap.host_names;
    let mut rows = Vec::new();

    for vm in &snap.vms {
        let Some(ctx) = VmContext::from(vm, hosts)? else {
            continue;
        };

        for disk in vm.array_prop("guest.disk") {
            let capacity = disk.text_at("capacity").and_then(|v| v.parse::<i64>().ok());
            let free = disk.text_at("freeSpace").and_then(|v| v.parse::<i64>().ok());
            // Consumed is derived: vCenter reports capacity and free, not used.
            let consumed = match (capacity, free) {
                (Some(c), Some(f)) => Some(c - f),
                _ => None,
            };
            let free_pct = match (capacity, free) {
                (Some(c), Some(f)) if c > 0 => {
                    Some((f as f64 / c as f64 * 10000.0).round() / 100.0)
                }
                _ => None,
            };

            rows.push((
                vm.moref.clone(),
                vec![
                    Cell::Text(ctx.name.clone()),
                    Cell::opt_text(ctx.power_state.clone()),
                    Cell::opt_bool(ctx.template),
                    Cell::opt_text(disk.text_at("diskPath").filter(|s| !s.is_empty())),
                    Cell::opt_text(disk.text_at("filesystemType").filter(|s| !s.is_empty())),
                    Cell::opt_num(capacity.map(to_mib)),
                    Cell::opt_num(consumed.map(to_mib)),
                    Cell::opt_num(free.map(to_mib)),
                    Cell::opt_num(free_pct),
                    Cell::opt_text(ctx.host.clone()),
                    Cell::opt_text(ctx.annotation.clone()),
                ],
            ));
        }
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vPartition",
    columns,
    vm_props: &[VM_CONTEXT_PROPS, VM_PROPS],
    host_props: &[],
    dvs_props: &[],
    dvpg_props: &[],
    source: RowSource::Vm,
    rows,
};

pub async fn fetch_vpartition_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_snapshot, cells, col};

    fn at(row: &[Cell], label: &str) -> Cell {
        row[col(&columns(), label)].clone()
    }

    /// Only VMs whose guest reports filesystems contribute rows; the count is
    /// per filesystem, not per VM.
    #[test]
    fn one_row_per_reported_filesystem() {
        let snap = captured_snapshot();
        let expected: usize = snap.vms.iter().map(|v| v.array_prop("guest.disk").len()).sum();
        let rows = cells(rows(&snap).expect("named VMs"));
        assert_eq!(rows.len(), expected);
    }

    /// vCenter reports capacity and free space in bytes; RVTools' columns are
    /// MiB, and Consumed is derived rather than reported.
    #[test]
    fn consumed_is_capacity_minus_free_and_units_are_mib() {
        let snap = captured_snapshot();
        let rows = cells(rows(&snap).expect("named VMs"));
        let Some(r) = rows.iter().find(|r| matches!(at(r, "Capacity MiB"), Cell::Number(_))) else {
            return; // no guest filesystems in the corpus; nothing to assert
        };
        let (cap, used, free) = (at(r, "Capacity MiB"), at(r, "Consumed MiB"), at(r, "Free MiB"));
        if let (Cell::Number(c), Cell::Number(u), Cell::Number(f)) = (cap, used, free) {
            assert!((c - (u + f)).abs() < 0.05, "capacity {c} should be used {u} + free {f}");
        }
    }

    /// A filesystem of zero capacity would make the percentage a division by
    /// zero; it stays empty rather than becoming 0 %.
    #[test]
    fn free_percent_is_empty_without_a_capacity() {
        let snap = captured_snapshot();
        let rows = cells(rows(&snap).expect("named VMs"));
        for r in &rows {
            if matches!(at(r, "Capacity MiB"), Cell::Empty) {
                assert!(matches!(at(r, "Free %"), Cell::Empty));
            }
        }
    }
}
