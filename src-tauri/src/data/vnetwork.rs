//! vNetwork — one row per virtual NIC.
//!
//! Reads `config.hardware.device` from the shared VM snapshot, the same array
//! vDisk and vCD read, so it adds no inventory walk.
//!
//! Two shapes here were confirmed against the live vCenter first:
//!
//! - A NIC's network is named three different ways depending on the backing.
//!   A distributed-port backing gives only `port/portgroupKey`, a moref such as
//!   `dvportgroup-9335`, which the inventory index resolves to a name. A
//!   standard backing gives `deviceName` directly, and may also carry a
//!   `network` moref. All three appear in the lab (174 / 56 / 4).
//! - IPv4 addresses live on `guest.net`, not on the device, and are matched to
//!   a NIC by `deviceConfigId`, which equals the device `key`. `guest.net`
//!   needs VMware Tools, so a NIC on a VM without Tools has no address — empty
//!   rather than wrong.
//!
//! `Network` is empty for 56 of the lab's 234 NICs, and that is the data, not
//! a parsing failure: those NICs use `VirtualEthernetCardLegacyNetworkBackingInfo`
//! whose `deviceName` comes back empty. Their VMs do report `guest.net`, but the
//! guest names none of those NICs either, so no source supplies a name. Measured,
//! not assumed.
//!
//! `Switch` is not implemented. A distributed-port backing identifies its
//! switch by `switchUuid`, and resolving that to a name needs
//! `DistributedVirtualSwitch`, which is Phase 2's dvSwitch work. An empty
//! column would read as a parsing failure, so the column is absent instead.

use super::common::{VmContext, VM_CONTEXT_PROPS};
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::xml::Element;
use crate::vcenter::VCenterConnection;
use std::collections::HashMap;

/// `VirtualMachine` properties this sheet reads.
pub const VM_PROPS: &[&str] = &["config.hardware.device", "guest.net"];

/// Concrete `VirtualEthernetCard` subclasses. vim25 names array elements after
/// the declared field type (`VirtualDevice`), so the concrete type is in
/// `xsi:type` and a NIC is recognised by its prefix.
fn is_ethernet_card(xsi_type: &str) -> bool {
    matches!(
        xsi_type,
        "VirtualVmxnet"
            | "VirtualVmxnet2"
            | "VirtualVmxnet3"
            | "VirtualVmxnet3Vrdma"
            | "VirtualE1000"
            | "VirtualE1000e"
            | "VirtualPCNet32"
            | "VirtualSriovEthernetCard"
    )
}

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("VM"),
        Column::text("Powerstate"),
        Column::bool("Template"),
        Column::text("NIC label"),
        Column::text("Adapter"),
        Column::text("Network"),
        Column::bool("Connected"),
        Column::bool("Starts Connected"),
        Column::text("Mac Address"),
        Column::text("Type"),
        Column::text("IPv4 Address"),
        Column::text("Host"),
        Column::text("Annotation"),
    ]
}

/// Device key → the first IPv4 address the guest reports for it.
///
/// `guest.net` is a `GuestNicInfo[]`, so its elements carry the type name;
/// `ipAddress` repeats inside each one under the field name. `deviceConfigId`
/// is the device `key`, which is how a guest NIC is tied back to a device.
fn guest_ipv4_by_device(vm: &crate::vcenter::soap::ManagedObject) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for nic in vm.array_prop("guest.net") {
        let Some(key) = nic.text_at("deviceConfigId").filter(|s| !s.is_empty()) else {
            continue;
        };
        let ipv4 = nic
            .children_named("ipAddress")
            .map(|e| e.text.clone())
            .find(|a| !a.is_empty() && a.contains('.') && !a.contains(':'));
        if let Some(ip) = ipv4 {
            out.entry(key).or_insert(ip);
        }
    }
    out
}

/// The network a NIC is attached to, however its backing names it.
fn network_name(backing: Option<&Element>, snap: &InventorySnapshot) -> Option<String> {
    let backing = backing?;
    // Distributed port: only a portgroup moref, resolved through the index.
    if let Some(key) = backing.text_at("port/portgroupKey").filter(|s| !s.is_empty()) {
        return snap.paths.name_of(&key).or(Some(key));
    }
    // Standard/legacy backing: the name is on the device.
    if let Some(name) = backing.text_at("deviceName").filter(|s| !s.is_empty()) {
        return Some(name);
    }
    // Some standard backings carry only a Network moref.
    let moref = backing.child("network").map(|e| e.text.clone()).filter(|s| !s.is_empty())?;
    snap.paths.name_of(&moref).or(Some(moref))
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let hosts = &snap.host_names;
    let mut rows = Vec::new();

    for vm in &snap.vms {
        let Some(ctx) = VmContext::from(vm, hosts)? else {
            continue;
        };
        let ips = guest_ipv4_by_device(vm);

        for nic in vm
            .array_prop("config.hardware.device")
            .into_iter()
            .filter(|d| d.xsi_type.as_deref().is_some_and(is_ethernet_card))
        {
            let key = nic.text_at("key").unwrap_or_default();
            let backing = nic.child("backing");

            rows.push((
                vm.moref.clone(),
                vec![
                    Cell::Text(ctx.name.clone()),
                    Cell::opt_text(ctx.power_state.clone()),
                    Cell::opt_bool(ctx.template),
                    Cell::opt_text(nic.text_at("deviceInfo/label")),
                    Cell::opt_text(nic.xsi_type.clone()),
                    Cell::opt_text(network_name(backing, snap)),
                    Cell::opt_bool(
                        nic.text_at("connectable/connected").map(|v| v == "true"),
                    ),
                    Cell::opt_bool(
                        nic.text_at("connectable/startConnected").map(|v| v == "true"),
                    ),
                    Cell::opt_text(nic.text_at("macAddress").filter(|s| !s.is_empty())),
                    // How the MAC was assigned: "manual", "generated", "assigned".
                    Cell::opt_text(nic.text_at("addressType").filter(|s| !s.is_empty())),
                    Cell::opt_text(ips.get(&key).cloned()),
                    Cell::opt_text(ctx.host.clone()),
                    Cell::opt_text(ctx.annotation.clone()),
                ],
            ));
        }
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vNetwork",
    columns,
    vm_props: &[VM_CONTEXT_PROPS, VM_PROPS],
    host_props: &[],
    dvs_props: &[],
    dvpg_props: &[],
    source: RowSource::Vm,
    rows,
};

pub async fn fetch_vnetwork_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_snapshot, cells, col};

    fn text_at_col(row: &[Cell], label: &str) -> Option<String> {
        match &row[col(&columns(), label)] {
            Cell::Text(s) => Some(s.clone()),
            _ => None,
        }
    }

    #[test]
    fn every_ethernet_card_in_the_capture_becomes_a_row() {
        let rows = cells(rows(&captured_snapshot()).expect("named VMs"));
        // The four captured VMs carry one NIC each.
        assert_eq!(rows.len(), 4);
    }

    /// The NIC lives in the same `VirtualDevice[]` as disks, controllers, the
    /// keyboard and the video card. Only ethernet cards may become rows.
    #[test]
    fn non_nic_devices_in_the_same_array_are_not_rows() {
        let snap = captured_snapshot();
        let devices = snap.vms[0].array_prop("config.hardware.device");
        assert!(devices.len() > 4, "capture carries many device types");
        let rows = cells(rows(&snap).expect("named VMs"));
        assert!(rows.len() < devices.len());
    }

    /// A distributed-port backing names its portgroup only by moref. Resolving
    /// it through the inventory index is what turns `dvportgroup-…` into a name.
    #[test]
    fn a_distributed_port_backing_resolves_to_a_network_name() {
        let rows = cells(rows(&captured_snapshot()).expect("named VMs"));
        let networks: Vec<_> = rows.iter().filter_map(|r| text_at_col(r, "Network")).collect();
        assert!(!networks.is_empty(), "at least one NIC should name a network");
        assert!(
            !networks.iter().any(|n| n.starts_with("dvportgroup-")),
            "portgroup morefs should be resolved to names, got {networks:?}"
        );
    }

    #[test]
    fn adapter_and_mac_come_off_the_device() {
        let rows = cells(rows(&captured_snapshot()).expect("named VMs"));
        let adapters: Vec<_> = rows.iter().filter_map(|r| text_at_col(r, "Adapter")).collect();
        assert!(
            adapters.iter().all(|a| a.starts_with("Virtual")),
            "adapter should be the concrete xsi:type, got {adapters:?}"
        );
        assert!(rows.iter().any(|r| text_at_col(r, "Mac Address").is_some()));
    }

    /// `Switch` needs DistributedVirtualSwitch, which is Phase 2. The column is
    /// absent rather than present and always empty.
    #[test]
    fn switch_is_not_claimed() {
        let labels: Vec<String> = columns().into_iter().map(|c| c.label).collect();
        assert!(!labels.iter().any(|l| l == "Switch"));
    }
}
