//! vCPU — one row per VM, CPU sizing and scheduling.
//!
//! Reads the VM snapshot every other VM-derived sheet reads, so it adds no
//! inventory walk. Every property path below was queried against the live
//! vCenter before it was written here.

use super::common::{VmContext, VM_CONTEXT_PROPS};
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

/// `VirtualMachine` properties this sheet reads.
pub const VM_PROPS: &[&str] = &[
    "config.hardware.numCPU",
    "config.hardware.numCoresPerSocket",
    "summary.runtime.maxCpuUsage",
    "summary.quickStats.overallCpuUsage",
    "summary.quickStats.staticCpuEntitlement",
    "summary.quickStats.distributedCpuEntitlement",
    "config.cpuAllocation.shares.level",
    "config.cpuAllocation.shares.shares",
    "config.cpuAllocation.reservation",
    "config.cpuAllocation.limit",
    "config.cpuHotAddEnabled",
    "config.cpuHotRemoveEnabled",
];

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("VM"),
        Column::text("Powerstate"),
        Column::bool("Template"),
        Column::number("CPUs"),
        Column::number("Sockets"),
        Column::number("Cores p/s"),
        Column::number("Max"),
        Column::number("Overall"),
        Column::text("Level"),
        Column::number("Shares"),
        Column::number("Reservation"),
        Column::number("Entitlement"),
        Column::number("DRS Entitlement"),
        Column::number("Limit"),
        Column::bool("Hot Add"),
        Column::bool("Hot Remove"),
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

        let cpus = vm.i64_prop("config.hardware.numCPU");
        let cores_per_socket = vm.i64_prop("config.hardware.numCoresPerSocket");
        // vCenter reports cores per socket, not sockets; RVTools shows both.
        let sockets = match (cpus, cores_per_socket) {
            (Some(c), Some(cps)) if cps > 0 => Some(c / cps),
            _ => None,
        };

        rows.push((
            vm.moref.clone(),
            vec![
                Cell::Text(ctx.name.clone()),
                Cell::opt_text(ctx.power_state.clone()),
                Cell::opt_bool(ctx.template),
                Cell::opt_num(cpus.map(|v| v as f64)),
                Cell::opt_num(sockets.map(|v| v as f64)),
                Cell::opt_num(cores_per_socket.map(|v| v as f64)),
                // MHz. A powered-off VM reports no maximum, and 0 would be a lie.
                Cell::opt_num(vm.i64_prop("summary.runtime.maxCpuUsage").map(|v| v as f64)),
                Cell::opt_num(
                    vm.i64_prop("summary.quickStats.overallCpuUsage").map(|v| v as f64),
                ),
                Cell::opt_text(vm.str_prop("config.cpuAllocation.shares.level")),
                Cell::opt_num(vm.i64_prop("config.cpuAllocation.shares.shares").map(|v| v as f64)),
                Cell::opt_num(vm.i64_prop("config.cpuAllocation.reservation").map(|v| v as f64)),
                Cell::opt_num(
                    vm.i64_prop("summary.quickStats.staticCpuEntitlement").map(|v| v as f64),
                ),
                Cell::opt_num(
                    vm.i64_prop("summary.quickStats.distributedCpuEntitlement").map(|v| v as f64),
                ),
                // -1 is vCenter's "unlimited"; RVTools shows it verbatim.
                Cell::opt_num(vm.i64_prop("config.cpuAllocation.limit").map(|v| v as f64)),
                Cell::opt_bool(vm.bool_prop("config.cpuHotAddEnabled")),
                Cell::opt_bool(vm.bool_prop("config.cpuHotRemoveEnabled")),
                Cell::opt_text(ctx.host.clone()),
                Cell::opt_text(ctx.annotation.clone()),
            ],
        ));
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vCPU",
    columns,
    vm_props: &[VM_CONTEXT_PROPS, VM_PROPS],
    host_props: &[],
    source: RowSource::Vm,
    rows,
};

pub async fn fetch_vcpu_all(
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

    /// vCenter reports cores per socket; sockets is derived. The captured VM
    /// has 2 CPUs at 2 cores per socket, so exactly one socket.
    #[test]
    fn sockets_are_derived_from_cpus_and_cores_per_socket() {
        let rows = cells(rows(&captured_snapshot()).expect("named VMs"));
        let r = row_for(&rows, "k8s-controller-01");
        let at = |l: &str| r[col(&columns(), l)].clone();
        assert!(matches!(at("CPUs"), Cell::Number(n) if n == 2.0));
        assert!(matches!(at("Cores p/s"), Cell::Number(n) if n == 2.0));
        assert!(matches!(at("Sockets"), Cell::Number(n) if n == 1.0));
    }

    #[test]
    fn allocation_columns_come_from_the_capture() {
        let rows = cells(rows(&captured_snapshot()).expect("named VMs"));
        let r = row_for(&rows, "appliance01");
        let at = |l: &str| r[col(&columns(), l)].clone();
        assert!(matches!(at("Level"), Cell::Text(ref s) if s == "normal"));
        assert!(matches!(at("Shares"), Cell::Number(n) if n > 0.0));
        // -1 is vCenter's "unlimited" and is shown as-is rather than blanked.
        assert!(matches!(at("Limit"), Cell::Number(n) if n == -1.0));
    }

    /// A powered-off VM reports no `summary.runtime.maxCpuUsage`. Empty and
    /// zero are different facts, so the cell stays empty.
    #[test]
    fn a_powered_off_vm_has_no_max() {
        let rows = cells(rows(&captured_snapshot()).expect("named VMs"));
        let r = row_for(&rows, "Windows Server 2025");
        assert!(matches!(r[col(&columns(), "Powerstate")], Cell::Text(ref s) if s == "poweredOff"));
        assert!(matches!(r[col(&columns(), "Max")], Cell::Empty));
    }
}
