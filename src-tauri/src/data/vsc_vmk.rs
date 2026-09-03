//! vSC_VMK — one row per VMkernel port.
//!
//! Reads `config.network.vnic` off the shared host snapshot. These are the
//! host's own IP interfaces — management, vMotion, vSAN, and the NSX ones —
//! not virtual-machine NICs, which are vNetwork.
//!
//! The port group is named two different ways. A VMkernel port on a standard
//! switch names it directly in `portgroup`; one on a distributed switch leaves
//! that empty and instead carries
//! `spec/distributedVirtualPort/portgroupKey`, a moref the inventory index
//! resolves. All six ports per host in the reference lab are the distributed
//! kind, and two of them (the NSX `vxlan` and `hyperbus` stacks) sit on no port
//! group at all.

use super::hostnet::HOST_NET_PROPS;
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

pub const HOST_PROPS: &[&str] = HOST_NET_PROPS;

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("Host"),
        Column::text("Port Group"),
        Column::text("Device"),
        Column::text("Mac Address"),
        Column::bool("DHCP"),
        Column::text("IP Address"),
        Column::text("Subnet mask"),
        Column::text("Gateway"),
        Column::number("MTU"),
        Column::text("TCP/IP Stack"),
    ]
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let mut rows = Vec::new();

    for host in &snap.hosts {
        let Some(name) = host.str_prop("name") else {
            return Err(format!("HostSystem {} returned no name property", host.moref));
        };

        for vmk in host.array_prop("config.network.vnic") {
            // Standard switch names the port group inline; a distributed one
            // gives a moref that has to be looked up.
            let portgroup = vmk
                .text_at("portgroup")
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    vmk.text_at("spec/distributedVirtualPort/portgroupKey")
                        .filter(|s| !s.is_empty())
                        .map(|key| snap.paths.name_of(&key).unwrap_or(key))
                });

            rows.push((
                host.moref.clone(),
                vec![
                    Cell::Text(name.clone()),
                    Cell::opt_text(portgroup),
                    Cell::opt_text(vmk.text_at("device").filter(|s| !s.is_empty())),
                    Cell::opt_text(vmk.text_at("spec/mac").filter(|s| !s.is_empty())),
                    Cell::opt_bool(vmk.text_at("spec/ip/dhcp").map(|v| v == "true")),
                    Cell::opt_text(vmk.text_at("spec/ip/ipAddress").filter(|s| !s.is_empty())),
                    Cell::opt_text(vmk.text_at("spec/ip/subnetMask").filter(|s| !s.is_empty())),
                    Cell::opt_text(
                        vmk.text_at("spec/ipRouteSpec/ipRouteConfig/defaultGateway")
                            .filter(|s| !s.is_empty()),
                    ),
                    Cell::opt_num(vmk.text_at("spec/mtu").and_then(|v| v.parse::<f64>().ok())),
                    // Which TCP/IP stack the port belongs to: defaultTcpipStack,
                    // vmotion, vxlan, hyperbus. RVTools has no column for this,
                    // but without it the NSX ports are indistinguishable.
                    Cell::opt_text(
                        vmk.text_at("spec/netStackInstanceKey").filter(|s| !s.is_empty()),
                    ),
                ],
            ));
        }
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vSC_VMK",
    columns,
    vm_props: &[],
    host_props: &[HOST_PROPS],
    dvs_props: &[],
    dvpg_props: &[],
    source: RowSource::Host,
    rows,
};

pub async fn fetch_vsc_vmk_all(
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
    fn one_row_per_vmkernel_port() {
        let snap = captured_snapshot();
        let expected: usize =
            snap.hosts.iter().map(|h| h.array_prop("config.network.vnic").len()).sum();
        let rows = cells(rows(&snap).expect("named host"));
        assert_eq!(rows.len(), expected);
    }

    #[test]
    fn addressing_comes_off_the_spec() {
        let rows = cells(rows(&captured_snapshot()).expect("named host"));
        let Some(r) = rows.iter().find(|r| !matches!(at(r, "IP Address"), Cell::Empty)) else {
            return;
        };
        assert!(matches!(at(r, "Subnet mask"), Cell::Text(_)));
        assert!(matches!(at(r, "MTU"), Cell::Number(_)));
        assert!(matches!(at(r, "Device"), Cell::Text(ref s) if s.starts_with("vmk")));
    }

    /// A distributed-switch VMkernel port names its port group only by moref.
    /// Any row that does name one must show a name, never `dvportgroup-…`.
    #[test]
    fn a_port_group_is_never_left_as_a_moref() {
        let rows = cells(rows(&captured_snapshot()).expect("named host"));
        for r in &rows {
            if let Cell::Text(pg) = at(r, "Port Group") {
                assert!(
                    !pg.starts_with("dvportgroup-"),
                    "port group should resolve to a name, got {pg:?}"
                );
            }
        }
    }
}
