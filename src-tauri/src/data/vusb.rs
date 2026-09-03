//! vUSB — one row per USB device attached to a VM.
//!
//! Reads `config.hardware.device` from the shared VM snapshot, so it adds no
//! inventory walk.
//!
//! Controllers are not USB devices and are deliberately not rows: RVTools' vUSB
//! inventories attached devices, and listing every `VirtualUSBController` would
//! put a row on each VM that merely has a USB bus.
//!
//! The lab originally had controllers but no `VirtualUSB` at all, so this sheet
//! had nothing to parse. A VM carrying one was created for the purpose
//! (`sttools-fixture-01`, see `docs/LAB-ENVIRONMENT.md`), and the columns below
//! are what a real device actually returned.
//!
//! RVTools' `Family`, `Speed`, `EHCI enabled` and `Auto connect` are still
//! **not** implemented: the captured device carries `vendor`, `product`,
//! `connected`, `controllerKey` and `unitNumber`, but none of those four. They
//! appear to require a device that is actually attached and connected, which
//! this one is not, so writing those paths would still be guesswork.

use super::common::{VmContext, VM_CONTEXT_PROPS};
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

/// `VirtualMachine` properties this sheet reads. The same array vDisk,
/// vNetwork and vCD read.
pub const VM_PROPS: &[&str] = &["config.hardware.device"];

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("VM"),
        Column::text("Powerstate"),
        Column::bool("Template"),
        Column::text("Device Node"),
        Column::text("Device Type"),
        Column::bool("Connected"),
        Column::number("Unit number"),
        Column::text("Summary"),
        Column::text("Host"),
        Column::text("Annotation"),
    ]
}

/// Attached USB devices, not the controllers they hang off.
fn is_usb_device(xsi_type: &str) -> bool {
    xsi_type == "VirtualUSB"
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let hosts = &snap.host_names;
    let mut rows = Vec::new();

    for vm in &snap.vms {
        let Some(ctx) = VmContext::from(vm, hosts)? else {
            continue;
        };

        for usb in vm
            .array_prop("config.hardware.device")
            .into_iter()
            .filter(|d| d.xsi_type.as_deref().is_some_and(is_usb_device))
        {
            rows.push((
                vm.moref.clone(),
                vec![
                    Cell::Text(ctx.name.clone()),
                    Cell::opt_text(ctx.power_state.clone()),
                    Cell::opt_bool(ctx.template),
                    Cell::opt_text(usb.text_at("deviceInfo/label")),
                    Cell::opt_text(usb.child("backing").and_then(|b| b.xsi_type.clone())),
                    // A VirtualUSB carries `connected` directly on the device,
                    // not inside a `connectable` block the way CD-ROMs and NICs
                    // do. Reading `connectable/connected` here silently yields
                    // an empty cell; the captured device is what showed that.
                    Cell::opt_bool(
                        usb.text_at("connected")
                            .or_else(|| usb.text_at("connectable/connected"))
                            .map(|v| v == "true"),
                    ),
                    Cell::opt_num(
                        usb.text_at("unitNumber").and_then(|v| v.parse::<f64>().ok()),
                    ),
                    Cell::opt_text(usb.text_at("deviceInfo/summary").filter(|s| !s.is_empty())),
                    Cell::opt_text(ctx.host.clone()),
                    Cell::opt_text(ctx.annotation.clone()),
                ],
            ));
        }
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vUSB",
    columns,
    vm_props: &[VM_CONTEXT_PROPS, VM_PROPS],
    host_props: &[],
    source: RowSource::Vm,
    rows,
};

pub async fn fetch_vusb_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_snapshot, cells, col};

    /// The corpus carries exactly one `VirtualUSB`, on the VM built for it.
    #[test]
    fn a_captured_usb_device_becomes_a_row() {
        let rows = cells(rows(&captured_snapshot()).expect("named VMs"));
        assert_eq!(rows.len(), 1);
        let at = |l: &str| rows[0][col(&columns(), l)].clone();
        assert!(matches!(at("VM"), Cell::Text(ref s) if s == "sttools-fixture-01"));
        assert!(matches!(at("Device Node"), Cell::Text(ref s) if s.starts_with("USB")));
        assert!(
            matches!(at("Device Type"), Cell::Text(ref s) if s == "VirtualUSBRemoteHostBackingInfo")
        );
        assert!(matches!(at("Connected"), Cell::Bool(false)));
    }

    /// The same VM has a USB *controller* alongside the device. Only the device
    /// is a row: a controller is a bus, not something anyone plugged in.
    #[test]
    fn the_controller_on_that_vm_is_not_also_a_row() {
        let snap = captured_snapshot();
        let vm = snap
            .vms
            .iter()
            .find(|v| v.str_prop("name").as_deref() == Some("sttools-fixture-01"))
            .expect("fixture VM present");
        let types: Vec<_> = vm
            .array_prop("config.hardware.device")
            .iter()
            .filter_map(|d| d.xsi_type.clone())
            .collect();
        assert!(types.iter().any(|t| t == "VirtualUSBController"));
        assert!(types.iter().any(|t| t == "VirtualUSB"));
        assert_eq!(cells(rows(&snap).expect("named VMs")).len(), 1);
    }

    /// A USB controller is not a USB device. The lab has controllers but no
    /// devices, and treating the former as the latter would put a row on every
    /// VM that merely has a USB bus.
    #[test]
    fn usb_controllers_are_not_rows() {
        assert!(!is_usb_device("VirtualUSBController"));
        assert!(!is_usb_device("VirtualUSBXHCIController"));
        assert!(is_usb_device("VirtualUSB"));
    }

    /// RVTools' Family / Speed / EHCI enabled / Auto connect are absent by
    /// choice: their properties have never been seen in a live response, and an
    /// always-empty column reads as a parsing bug.
    #[test]
    fn unverified_usb_columns_are_not_claimed() {
        let labels: Vec<String> = columns().into_iter().map(|c| c.label).collect();
        for absent in ["Family", "Speed", "EHCI enabled", "Auto connect"] {
            assert!(!labels.iter().any(|l| l == absent), "{absent} is not verified");
        }
    }
}
