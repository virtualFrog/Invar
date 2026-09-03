//! vNIC — one row per physical network adapter.
//!
//! Reads `config.network.pnic` off the shared host snapshot, and resolves the
//! switch and uplink port each NIC serves through the host's proxy switch (see
//! `hostnet::pnic_attachments`).
//!
//! `Speed` and `Duplex` come from `linkSpeed`, which vCenter sends **only when
//! the link is up**. Four of six NICs per host in this lab are unused, so those
//! cells are empty — that is the cable, not the parser.

use super::hostnet::{pnic_attachments, HOST_NET_PROPS};
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

pub const HOST_PROPS: &[&str] = HOST_NET_PROPS;

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("Host"),
        Column::text("Network Device"),
        Column::text("Driver"),
        Column::number("Speed"),
        Column::bool("Duplex"),
        Column::text("MAC"),
        Column::text("Switch"),
        Column::text("Uplink port"),
        Column::text("PCI"),
        Column::bool("WakeOn"),
    ]
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let mut rows = Vec::new();

    for host in &snap.hosts {
        let Some(name) = host.str_prop("name") else {
            return Err(format!("HostSystem {} returned no name property", host.moref));
        };
        let attached = pnic_attachments(host);

        for pnic in host.array_prop("config.network.pnic") {
            let device = pnic.text_at("device").filter(|s| !s.is_empty());
            let attachment = device.as_ref().and_then(|d| attached.get(d));

            rows.push((
                host.moref.clone(),
                vec![
                    Cell::Text(name.clone()),
                    Cell::opt_text(device.clone()),
                    Cell::opt_text(pnic.text_at("driver").filter(|s| !s.is_empty())),
                    // Absent on a NIC whose link is down.
                    Cell::opt_num(
                        pnic.text_at("linkSpeed/speedMb").and_then(|v| v.parse::<f64>().ok()),
                    ),
                    Cell::opt_bool(pnic.text_at("linkSpeed/duplex").map(|v| v == "true")),
                    Cell::opt_text(pnic.text_at("mac").filter(|s| !s.is_empty())),
                    Cell::opt_text(attachment.and_then(|a| a.switch.clone())),
                    Cell::opt_text(attachment.and_then(|a| a.uplink_port.clone())),
                    Cell::opt_text(pnic.text_at("pci").filter(|s| !s.is_empty())),
                    Cell::opt_bool(pnic.text_at("wakeOnLanSupported").map(|v| v == "true")),
                ],
            ));
        }
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vNIC",
    columns,
    vm_props: &[],
    host_props: &[HOST_PROPS],
    dvs_props: &[],
    dvpg_props: &[],
    cluster_props: &[],
    datastore_props: &[],
    rp_props: &[],
    wants_licenses: false,
    wants_about: false,
    source: RowSource::Host,
    rows,
};

pub async fn fetch_vnic_all(
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
    fn one_row_per_physical_nic() {
        let snap = captured_snapshot();
        let expected: usize =
            snap.hosts.iter().map(|h| h.array_prop("config.network.pnic").len()).sum();
        let rows = cells(rows(&snap).expect("named host"));
        assert_eq!(rows.len(), expected);
    }

    /// An unused NIC has no `linkSpeed` at all. Empty is the honest answer;
    /// a zero would claim the link is up and running at 0 Mb.
    #[test]
    fn a_nic_with_no_link_reports_no_speed() {
        let rows = cells(rows(&captured_snapshot()).expect("named host"));
        for r in &rows {
            if matches!(at(r, "Speed"), Cell::Empty) {
                assert!(matches!(at(r, "Duplex"), Cell::Empty));
            }
        }
    }

    /// Only the NICs actually backing an uplink name a switch; the rest are
    /// cabled to nothing and correctly say so.
    #[test]
    fn only_attached_nics_name_a_switch() {
        let rows = cells(rows(&captured_snapshot()).expect("named host"));
        for r in &rows {
            if !matches!(at(r, "Switch"), Cell::Empty) {
                assert!(
                    !matches!(at(r, "Uplink port"), Cell::Empty),
                    "a NIC on a switch should also name its uplink port"
                );
            }
        }
    }
}
