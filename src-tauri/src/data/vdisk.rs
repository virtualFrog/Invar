//! vDisk — one row per virtual disk.
//!
//! Sourced from `config.hardware.device`, filtered to `xsi:type="VirtualDisk"`.
//! The array elements are named `<VirtualDevice>` after the field's declared
//! type; the concrete device type is only in `xsi:type`, so filtering on the
//! element name would silently yield nothing.

use super::common::{VmContext, VM_CONTEXT_PROPS};
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::xml::Element;
use crate::vcenter::VCenterConnection;
use std::collections::HashMap;

/// What this sheet reads beyond `common::VM_CONTEXT_PROPS`: the device array
/// the disks come out of.
pub const VM_PROPS: &[&str] = &["config.hardware.device"];

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("VM"),
        Column::text("Powerstate"),
        Column::bool("Template"),
        Column::text("Disk"),
        Column::number("Disk Key"),
        Column::text("Disk UUID"),
        Column::text("Disk Path"),
        Column::number("Capacity MiB"),
        Column::bool("Raw"),
        Column::text("Disk Mode"),
        Column::text("Sharing mode"),
        Column::bool("Thin"),
        Column::bool("Eagerly Scrub"),
        Column::bool("Split"),
        Column::bool("Write Through"),
        Column::text("Level"),
        Column::number("Shares"),
        Column::number("Reservation"),
        Column::number("Limit"),
        Column::text("Controller"),
        Column::number("Unit #"),
        Column::text("Raw LUN ID"),
        Column::text("Raw Comp. Mode"),
        Column::text("Host"),
        Column::text("Annotation"),
    ]
}

fn text(el: &Element, path: &str) -> Option<String> {
    el.text_at(path).filter(|s| !s.is_empty())
}

fn number(el: &Element, path: &str) -> Option<f64> {
    text(el, path)?.parse().ok()
}

fn boolean(el: &Element, path: &str) -> Option<bool> {
    match text(el, path)?.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Device key → label, so a disk's `controllerKey` can name its controller
/// ("SCSI controller 0").
fn controller_labels(devices: &[&Element]) -> HashMap<String, String> {
    devices
        .iter()
        .filter_map(|d| Some((d.text_at("key")?, d.text_at("deviceInfo/label")?)))
        .collect()
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let hosts = &snap.host_names;

    let mut rows = Vec::new();
    for vm in &snap.vms {
        let Some(ctx) = VmContext::from(vm, hosts)? else {
            continue;
        };
        let devices = vm.array_prop("config.hardware.device");
        let controllers = controller_labels(&devices);

        for disk in devices
            .iter()
            .filter(|d| d.xsi_type.as_deref() == Some("VirtualDisk"))
        {
            let backing = disk.child("backing");
            // A raw device mapping is backed by RawDiskMappingVer1BackingInfo
            // rather than a flat vmdk. None exist in the lab used for
            // development, so these columns are expected to be empty there.
            let is_raw = backing
                .and_then(|b| b.xsi_type.as_deref())
                .is_some_and(|t| t.starts_with("RawDiskMapping"));

            rows.push((vm.moref.clone(), vec![
                Cell::Text(ctx.name.clone()),
                Cell::opt_text(ctx.power_state.clone()),
                Cell::opt_bool(ctx.template),
                Cell::opt_text(text(disk, "deviceInfo/label")),
                Cell::opt_num(number(disk, "key")),
                Cell::opt_text(backing.and_then(|b| text(b, "uuid"))),
                Cell::opt_text(backing.and_then(|b| text(b, "fileName"))),
                // capacityInKB is KiB; RVTools' column is MiB.
                Cell::opt_num(number(disk, "capacityInKB").map(|kb| (kb / 1024.0 * 100.0).round() / 100.0)),
                Cell::Bool(is_raw),
                Cell::opt_text(backing.and_then(|b| text(b, "diskMode"))),
                Cell::opt_text(backing.and_then(|b| text(b, "sharing"))),
                Cell::opt_bool(backing.and_then(|b| boolean(b, "thinProvisioned"))),
                Cell::opt_bool(backing.and_then(|b| boolean(b, "eagerlyScrub"))),
                Cell::opt_bool(backing.and_then(|b| boolean(b, "split"))),
                Cell::opt_bool(backing.and_then(|b| boolean(b, "writeThrough"))),
                // Level/Shares/Reservation/Limit are the disk's storage I/O
                // allocation, not the device-level <shares> block above it.
                Cell::opt_text(text(disk, "storageIOAllocation/shares/level")),
                Cell::opt_num(number(disk, "storageIOAllocation/shares/shares")),
                Cell::opt_num(number(disk, "storageIOAllocation/reservation")),
                Cell::opt_num(number(disk, "storageIOAllocation/limit")),
                Cell::opt_text(
                    text(disk, "controllerKey").and_then(|k| controllers.get(&k).cloned()),
                ),
                Cell::opt_num(number(disk, "unitNumber")),
                Cell::opt_text(backing.and_then(|b| text(b, "lunUuid"))),
                Cell::opt_text(backing.and_then(|b| text(b, "compatibilityMode"))),
                Cell::opt_text(ctx.host.clone()),
                Cell::opt_text(ctx.annotation.clone()),
            ]));
        }
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vDisk",
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

pub async fn fetch_vdisk_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured, captured_snapshot, cells, col, VM_MULTI_DISK};

    fn cell(rows: &[Vec<Cell>], row: usize, label: &str) -> Cell {
        rows[row][col(&columns(), label)].clone()
    }

    fn text_of(c: &Cell) -> Option<&str> {
        match c {
            Cell::Text(s) => Some(s),
            _ => None,
        }
    }

    /// The whole sheet over real captures: four VMs carrying 6 + 3 + 1 + 1
    /// disks. A hand-written fragment would only ever prove we can parse the
    /// shape we imagined.
    #[test]
    fn every_virtual_disk_in_the_capture_becomes_a_row() {
        let rows = cells(rows(&captured_snapshot()).expect("captured VMs all have names"));
        assert_eq!(rows.len(), 11);
    }

    /// `config.hardware.device` is a `VirtualDevice[]`, so vim25 names its
    /// elements after the declared type and distinguishes the real one with
    /// `xsi:type`. The capture carries controllers, NICs, keyboard and video
    /// card alongside the disks; only the disks may become rows.
    #[test]
    fn non_disk_devices_in_the_same_array_are_not_rows() {
        let vm = captured(VM_MULTI_DISK);
        let devices = vm.array_prop("config.hardware.device");
        assert!(
            devices.len() > 6,
            "capture should carry more devices than just its 6 disks, got {}",
            devices.len()
        );
        let snap = crate::data::snapshot::InventorySnapshot::from_parts(vec![vm], Vec::new());
        assert_eq!(cells(rows(&snap).expect("named VM")).len(), 6);
    }

    /// capacityInKB is KiB; RVTools' column is MiB. 33554432 KiB = 32768 MiB.
    #[test]
    fn capacity_is_converted_from_kib_to_the_mib_column() {
        let snap =
            crate::data::snapshot::InventorySnapshot::from_parts(vec![captured(VM_MULTI_DISK)], Vec::new());
        let rows = cells(rows(&snap).expect("named VM"));
        assert_eq!(text_of(&cell(&rows, 0, "Disk")), Some("Hard disk 1"));
        assert!(matches!(cell(&rows, 0, "Capacity MiB"), Cell::Number(n) if n == 32768.0));
    }

    /// A disk's controller is resolved through `controllerKey` into the
    /// controller device's own label.
    #[test]
    fn a_disk_names_the_controller_it_hangs_off() {
        let snap =
            crate::data::snapshot::InventorySnapshot::from_parts(vec![captured(VM_MULTI_DISK)], Vec::new());
        let rows = cells(rows(&snap).expect("named VM"));
        let controller = cell(&rows, 0, "Controller");
        let controller = text_of(&controller).expect("controller resolves to a label");
        assert!(
            controller.contains("SCSI controller"),
            "expected a SCSI controller label, got {controller:?}"
        );
    }

    /// No RDM exists in the lab, so every captured disk is flat-file backed and
    /// the raw-mapping columns stay empty. Documented in
    /// `docs/LAB-ENVIRONMENT.md` as expected rather than a gap.
    #[test]
    fn flat_backed_disks_report_no_raw_mapping() {
        let snap =
            crate::data::snapshot::InventorySnapshot::from_parts(vec![captured(VM_MULTI_DISK)], Vec::new());
        let rows = cells(rows(&snap).expect("named VM"));
        for (i, _) in rows.iter().enumerate() {
            assert!(matches!(cell(&rows, i, "Raw"), Cell::Bool(false)));
            assert!(matches!(cell(&rows, i, "Raw LUN ID"), Cell::Empty));
            assert!(matches!(cell(&rows, i, "Raw Comp. Mode"), Cell::Empty));
        }
    }

    /// `storageIOAllocation` is nested inside the disk, so its `shares` block
    /// repeats the FIELD name rather than the type name -- the exception to the
    /// top-level array rule in CLAUDE.md. Reading it wrong yields empty cells,
    /// not an error, which is why this asserts against a real capture.
    #[test]
    fn storage_io_allocation_is_read_from_the_nested_shares_block() {
        let snap =
            crate::data::snapshot::InventorySnapshot::from_parts(vec![captured(VM_MULTI_DISK)], Vec::new());
        let rows = cells(rows(&snap).expect("named VM"));
        assert!(
            matches!(cell(&rows, 0, "Shares"), Cell::Number(n) if n > 0.0),
            "shares should come through as a number, got {:?}",
            cell(&rows, 0, "Shares")
        );
        assert!(text_of(&cell(&rows, 0, "Level")).is_some(), "level should be populated");
    }
}
