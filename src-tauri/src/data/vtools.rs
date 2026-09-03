//! vTools — one row per VM, VMware Tools state.
//!
//! Reads the shared VM snapshot, so it adds no inventory walk. Every property
//! path was queried live first.
//!
//! `Required Version` is not implemented: no property on `VirtualMachine`
//! observed against the live vCenter supplies it, and guessing one would give a
//! confidently wrong column.

use super::common::{VmContext, VM_CONTEXT_PROPS};
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

/// `VirtualMachine` properties this sheet reads.
pub const VM_PROPS: &[&str] = &[
    "config.version",
    "guest.toolsRunningStatus",
    "guest.toolsVersion",
    "guest.toolsVersionStatus2",
    "config.tools.toolsUpgradePolicy",
    "config.tools.syncTimeWithHost",
    "guest.appHeartbeatStatus",
    "guestHeartbeatStatus",
    "guest.guestKernelCrashed",
    "guest.guestOperationsReady",
    "guest.guestStateChangeSupported",
    "guest.interactiveGuestOperationsReady",
];

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("VM"),
        Column::text("Powerstate"),
        Column::bool("Template"),
        Column::text("VM Version"),
        Column::text("Tools"),
        Column::text("Tools Version"),
        Column::text("Upgradeable"),
        Column::text("Upgrade Policy"),
        Column::bool("Sync time"),
        Column::text("App status"),
        Column::text("Heartbeat status"),
        Column::bool("Kernel Crash state"),
        Column::bool("Operation Ready"),
        Column::bool("State change support"),
        Column::bool("Interactive Guest"),
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

        rows.push((
            vm.moref.clone(),
            vec![
                Cell::Text(ctx.name.clone()),
                Cell::opt_text(ctx.power_state.clone()),
                Cell::opt_bool(ctx.template),
                Cell::opt_text(vm.str_prop("config.version")),
                Cell::opt_text(vm.str_prop("guest.toolsRunningStatus")),
                Cell::opt_text(vm.str_prop("guest.toolsVersion")),
                // `toolsVersionStatus2` is the current form; the older
                // `toolsVersionStatus` is deprecated and reports the same thing.
                Cell::opt_text(vm.str_prop("guest.toolsVersionStatus2")),
                Cell::opt_text(vm.str_prop("config.tools.toolsUpgradePolicy")),
                Cell::opt_bool(vm.bool_prop("config.tools.syncTimeWithHost")),
                Cell::opt_text(vm.str_prop("guest.appHeartbeatStatus")),
                Cell::opt_text(vm.str_prop("guestHeartbeatStatus")),
                Cell::opt_bool(vm.bool_prop("guest.guestKernelCrashed")),
                Cell::opt_bool(vm.bool_prop("guest.guestOperationsReady")),
                Cell::opt_bool(vm.bool_prop("guest.guestStateChangeSupported")),
                Cell::opt_bool(vm.bool_prop("guest.interactiveGuestOperationsReady")),
                Cell::opt_text(ctx.host.clone()),
                Cell::opt_text(ctx.annotation.clone()),
            ],
        ));
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vTools",
    columns,
    vm_props: &[VM_CONTEXT_PROPS, VM_PROPS],
    host_props: &[],
    dvs_props: &[],
    dvpg_props: &[],
    cluster_props: &[],
    datastore_props: &[],
    rp_props: &[],
    wants_licenses: false,
    wants_about: false,
    wants_files: false,
    source: RowSource::Vm,
    rows,
};

pub async fn fetch_vtools_all(
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
    fn tools_columns_come_from_the_capture() {
        let rows = cells(rows(&captured_snapshot()).expect("named VMs"));
        let r = row_for(&rows, "appliance01");
        let at = |l: &str| r[col(&columns(), l)].clone();
        assert!(matches!(at("VM Version"), Cell::Text(ref s) if s == "vmx-13"));
        assert!(matches!(at("Tools"), Cell::Text(ref s) if s.starts_with("guestTools")));
        assert!(matches!(at("Tools Version"), Cell::Text(_)));
    }

    /// `Required Version` has no observed source property, so the sheet does
    /// not carry the column rather than carrying an always-empty one.
    #[test]
    fn required_version_is_not_claimed() {
        let labels: Vec<String> = columns().into_iter().map(|c| c.label).collect();
        assert!(!labels.iter().any(|l| l == "Required Version"));
    }
}
