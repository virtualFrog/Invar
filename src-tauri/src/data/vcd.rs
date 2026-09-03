//! vCD — one row per CD/DVD drive.
//!
//! Reads `config.hardware.device` from the shared VM snapshot, so it adds no
//! inventory walk. The device is recognised by `xsi:type == "VirtualCdrom"`,
//! confirmed live: 23 of them across the lab.
//!
//! `Device Type` is the backing's `xsi:type`, which is what actually says how
//! the drive is attached — `VirtualCdromAtapiBackingInfo` (host device),
//! `VirtualCdromIsoBackingInfo` (an ISO on a datastore),
//! `VirtualCdromRemotePassthroughBackingInfo` (client device). RVTools shows
//! the same distinction.

use super::common::{VmContext, VM_CONTEXT_PROPS};
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

/// `VirtualMachine` properties this sheet reads. The same array vDisk,
/// vNetwork and vUSB read.
pub const VM_PROPS: &[&str] = &["config.hardware.device"];

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("VM"),
        Column::text("Powerstate"),
        Column::bool("Template"),
        Column::text("Device Node"),
        Column::bool("Connected"),
        Column::bool("Starts Connected"),
        Column::text("Device Type"),
        Column::text("Summary"),
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

        for cd in vm
            .array_prop("config.hardware.device")
            .into_iter()
            .filter(|d| d.xsi_type.as_deref() == Some("VirtualCdrom"))
        {
            rows.push((
                vm.moref.clone(),
                vec![
                    Cell::Text(ctx.name.clone()),
                    Cell::opt_text(ctx.power_state.clone()),
                    Cell::opt_bool(ctx.template),
                    Cell::opt_text(cd.text_at("deviceInfo/label")),
                    Cell::opt_bool(cd.text_at("connectable/connected").map(|v| v == "true")),
                    Cell::opt_bool(cd.text_at("connectable/startConnected").map(|v| v == "true")),
                    // How it is backed, not what it is: the backing type is the
                    // useful distinction (host device vs ISO vs client device).
                    Cell::opt_text(cd.child("backing").and_then(|b| b.xsi_type.clone())),
                    Cell::opt_text(cd.text_at("deviceInfo/summary").filter(|s| !s.is_empty())),
                    Cell::opt_text(ctx.host.clone()),
                    Cell::opt_text(ctx.annotation.clone()),
                ],
            ));
        }
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vCD",
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
    source: RowSource::Vm,
    rows,
};

pub async fn fetch_vcd_all(
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

    #[test]
    fn one_row_per_cdrom_device() {
        let snap = captured_snapshot();
        let expected: usize = snap
            .vms
            .iter()
            .map(|v| {
                v.array_prop("config.hardware.device")
                    .iter()
                    .filter(|d| d.xsi_type.as_deref() == Some("VirtualCdrom"))
                    .count()
            })
            .sum();
        let rows = cells(rows(&snap).expect("named VMs"));
        assert_eq!(rows.len(), expected);
        assert!(expected > 0, "the corpus should contain a CD-ROM");
    }

    /// A disconnected drive is still a row. vHealth reports only connected
    /// drives; vCD inventories them all, which is a different question.
    #[test]
    fn a_disconnected_drive_is_still_inventoried() {
        let rows = cells(rows(&captured_snapshot()).expect("named VMs"));
        assert!(
            rows.iter().any(|r| matches!(at(r, "Connected"), Cell::Bool(false))),
            "the corpus has drives that are not connected"
        );
    }

    /// The backing type is what says how the drive is attached, and it is on
    /// the backing element rather than the device.
    #[test]
    fn device_type_is_the_backing_type() {
        let rows = cells(rows(&captured_snapshot()).expect("named VMs"));
        let types: Vec<_> = rows
            .iter()
            .filter_map(|r| match at(r, "Device Type") {
                Cell::Text(s) => Some(s),
                _ => None,
            })
            .collect();
        assert!(!types.is_empty());
        assert!(
            types.iter().all(|t| t.starts_with("VirtualCdrom")),
            "expected VirtualCdrom*BackingInfo, got {types:?}"
        );
    }
}
