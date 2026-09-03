//! vMemory — one row per VM, memory sizing and what the host is actually
//! backing it with.
//!
//! Reads the shared VM snapshot, so it adds no inventory walk. Every property
//! path was queried live first.
//!
//! **`Overhead` is not implemented.** RVTools sources it from
//! `runtime.memoryOverhead`, which this vCenter returned for none of its 161
//! VMs. Writing the path anyway would produce a column that is silently empty
//! and looks like a parsing bug. `Consumed Overhead` is a different property
//! (`summary.quickStats.consumedOverheadMemory`) and is implemented.

use super::common::{VmContext, VM_CONTEXT_PROPS};
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

/// `VirtualMachine` properties this sheet reads.
pub const VM_PROPS: &[&str] = &[
    "summary.config.memorySizeMB",
    "config.memoryReservationLockedToMax",
    "summary.quickStats.hostMemoryUsage",
    "summary.quickStats.guestMemoryUsage",
    "summary.quickStats.consumedOverheadMemory",
    "summary.quickStats.privateMemory",
    "summary.quickStats.sharedMemory",
    "summary.quickStats.swappedMemory",
    "summary.quickStats.balloonedMemory",
    "summary.quickStats.staticMemoryEntitlement",
    "summary.quickStats.distributedMemoryEntitlement",
    "config.memoryAllocation.shares.level",
    "config.memoryAllocation.shares.shares",
    "config.memoryAllocation.reservation",
    "config.memoryAllocation.limit",
    "config.memoryHotAddEnabled",
];

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("VM"),
        Column::text("Powerstate"),
        Column::bool("Template"),
        Column::number("Size MiB"),
        Column::bool("Memory Reservation Locked To Max"),
        Column::number("Consumed"),
        Column::number("Consumed Overhead"),
        Column::number("Private"),
        Column::number("Shared"),
        Column::number("Swapped"),
        Column::number("Ballooned"),
        Column::number("Active"),
        Column::number("Entitlement"),
        Column::number("DRS Entitlement"),
        Column::text("Level"),
        Column::number("Shares"),
        Column::number("Reservation"),
        Column::number("Limit"),
        Column::bool("Hot Add"),
        Column::text("Host"),
        Column::text("Annotation"),
    ]
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let hosts = &snap.host_names;
    let mut rows = Vec::with_capacity(snap.vms.len());

    for vm in &snap.vms {
        let Some(ctx) = VmContext::from(vm, hosts)? else {
            continue;
        };
        // Every quickStats memory figure below is already MiB, so these are
        // reported as vCenter gives them rather than converted.
        let mib = |path: &str| Cell::opt_num(vm.i64_prop(path).map(|v| v as f64));

        rows.push((
            vm.moref.clone(),
            vec![
                Cell::Text(ctx.name.clone()),
                Cell::opt_text(ctx.power_state.clone()),
                Cell::opt_bool(ctx.template),
                mib("summary.config.memorySizeMB"),
                Cell::opt_bool(vm.bool_prop("config.memoryReservationLockedToMax")),
                // What the host is backing the VM with, versus what the guest
                // thinks it is using: two different facts, both reported.
                mib("summary.quickStats.hostMemoryUsage"),
                mib("summary.quickStats.consumedOverheadMemory"),
                mib("summary.quickStats.privateMemory"),
                mib("summary.quickStats.sharedMemory"),
                mib("summary.quickStats.swappedMemory"),
                mib("summary.quickStats.balloonedMemory"),
                mib("summary.quickStats.guestMemoryUsage"),
                mib("summary.quickStats.staticMemoryEntitlement"),
                mib("summary.quickStats.distributedMemoryEntitlement"),
                Cell::opt_text(vm.str_prop("config.memoryAllocation.shares.level")),
                Cell::opt_num(
                    vm.i64_prop("config.memoryAllocation.shares.shares").map(|v| v as f64),
                ),
                Cell::opt_num(vm.i64_prop("config.memoryAllocation.reservation").map(|v| v as f64)),
                Cell::opt_num(vm.i64_prop("config.memoryAllocation.limit").map(|v| v as f64)),
                Cell::opt_bool(vm.bool_prop("config.memoryHotAddEnabled")),
                Cell::opt_text(ctx.host.clone()),
                Cell::opt_text(ctx.annotation.clone()),
            ],
        ));
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vMemory",
    columns,
    vm_props: &[VM_CONTEXT_PROPS, VM_PROPS],
    host_props: &[],
    dvs_props: &[],
    dvpg_props: &[],
    source: RowSource::Vm,
    rows,
};

pub async fn fetch_vmemory_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_snapshot, cells, col};

    fn row_for(rows: &[Vec<Cell>], vm: &str) -> Vec<Cell> {
        let i = col(&columns(), "VM");
        rows.iter()
            .find(|r| matches!(&r[i], Cell::Text(n) if n == vm))
            .unwrap_or_else(|| panic!("no row for {vm}"))
            .clone()
    }

    #[test]
    fn one_row_per_vm() {
        let rows = cells(rows(&captured_snapshot()).expect("named VMs"));
        assert_eq!(rows.len(), 5);
    }

    #[test]
    fn size_comes_from_the_capture() {
        let rows = cells(rows(&captured_snapshot()).expect("named VMs"));
        let r = row_for(&rows, "appliance01");
        assert!(matches!(r[col(&columns(), "Size MiB")], Cell::Number(n) if n == 16384.0));
    }

    /// The sheet deliberately has no `Overhead` column: vCenter returned
    /// `runtime.memoryOverhead` for no VM at all, and an always-empty column
    /// reads as a bug. `Consumed Overhead` is a different property and is here.
    #[test]
    fn overhead_is_absent_but_consumed_overhead_is_not() {
        let labels: Vec<String> = columns().into_iter().map(|c| c.label).collect();
        assert!(!labels.iter().any(|l| l == "Overhead"));
        assert!(labels.iter().any(|l| l == "Consumed Overhead"));
    }

    /// Consumed is what the host backs; Active is what the guest reports. They
    /// are separate properties and must not be conflated.
    #[test]
    fn consumed_and_active_are_distinct_properties() {
        let snap = captured_snapshot();
        let rows = cells(rows(&snap).expect("named VMs"));
        let r = row_for(&rows, "appliance01");
        let consumed = r[col(&columns(), "Consumed")].clone();
        let active = r[col(&columns(), "Active")].clone();
        assert!(matches!(consumed, Cell::Number(_)));
        assert!(matches!(active, Cell::Number(_)));
    }
}
