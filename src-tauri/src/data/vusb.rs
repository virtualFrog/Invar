//! vUSB — one row per USB device attached to a VM.
//!
//! Reads `config.hardware.device` from the shared VM snapshot, so it adds no
//! inventory walk.
//!
//! # This sheet is not verified against live data
//!
//! The lab has USB *controllers* (`VirtualUSBController` ×10,
//! `VirtualUSBXHCIController` ×3) but **no `VirtualUSB` devices at all**, so
//! this sheet produces zero rows there and its parsing has never run against a
//! real response. Controllers are not USB devices and are deliberately not
//! rows: RVTools' vUSB inventories attached devices, and listing controllers
//! would inflate the sheet with things nobody plugged in.
//!
//! Because of that, the columns here are restricted to fields carried by the
//! `VirtualDevice` base type, which *is* verified live across CD-ROM, floppy,
//! disk and NIC devices in this same array: `key`, `deviceInfo/label`,
//! `deviceInfo/summary`, `connectable/*`, `controllerKey` and `unitNumber`.
//!
//! RVTools' `Family`, `Speed`, `EHCI enabled` and `Auto connect` are **not**
//! implemented. They live on `VirtualUSB` itself, and no response containing
//! one has been observed. Writing those paths would break the project's first
//! ground rule and would most likely ship silently empty columns.

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
                    Cell::opt_bool(usb.text_at("connectable/connected").map(|v| v == "true")),
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
    use crate::data::snapshot::test_support::{captured_snapshot, cells};
    use crate::data::snapshot::InventorySnapshot;
    use crate::vcenter::soap::ManagedObject;
    use crate::vcenter::xml;

    /// No `VirtualUSB` device exists in the lab, so the captured corpus yields
    /// nothing. Asserted rather than assumed, so that if a capture ever does
    /// contain one this test says so.
    #[test]
    fn the_captured_corpus_has_no_usb_devices() {
        let rows = cells(rows(&captured_snapshot()).expect("named VMs"));
        assert!(rows.is_empty());
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

    /// Synthetic, and marked as such: no live response containing a
    /// `VirtualUSB` has been observed, so this fixture asserts the shape the
    /// vim25 schema documents rather than one that was captured. It uses only
    /// `VirtualDevice` base fields, which are verified live on other devices in
    /// the same array.
    #[test]
    fn a_usb_device_becomes_a_row_synthetic_shape() {
        let fragment = r#"<objects><obj type="VirtualMachine">vm-1</obj>
            <propSet><name>name</name><val>usb-vm</val></propSet>
            <propSet><name>config.hardware.device</name><val>
              <VirtualDevice xsi:type="VirtualUSBController">
                <key>7000</key><deviceInfo><label>USB controller</label></deviceInfo>
              </VirtualDevice>
              <VirtualDevice xsi:type="VirtualUSB">
                <key>7001</key>
                <deviceInfo><label>USB 1</label><summary>Generic USB device</summary></deviceInfo>
                <backing xsi:type="VirtualUSBRemoteHostBackingInfo"/>
                <connectable><connected>true</connected></connectable>
                <unitNumber>1</unitNumber>
              </VirtualDevice>
            </val></propSet></objects>"#;
        let vm = ManagedObject::from_element(&xml::parse(fragment).expect("fragment parses"));
        let snap = InventorySnapshot::from_parts(vec![vm], Vec::new());
        let rows = cells(rows(&snap).expect("named VM"));
        // The controller is skipped; only the device is a row.
        assert_eq!(rows.len(), 1);
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
