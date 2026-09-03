//! vHBA — one row per host bus adapter.
//!
//! Reads `config.storageDevice.hostBusAdapter` off the shared host snapshot.
//! The array is a `HostHostBusAdapter[]`, so its elements carry the *type* name
//! and the concrete adapter kind is in `xsi:type`
//! (`HostSerialAttachedHba` in this lab, an HPE Smart Array controller).

use super::hostnet::HOST_HBA_PROPS;
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

pub const HOST_PROPS: &[&str] = HOST_HBA_PROPS;

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("Host"),
        Column::text("Device"),
        Column::text("Type"),
        Column::text("Status"),
        Column::text("Bus"),
        Column::text("Pci"),
        Column::text("Driver"),
        Column::text("Model"),
        Column::text("WWN"),
    ]
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let mut rows = Vec::new();

    for host in &snap.hosts {
        let Some(name) = host.str_prop("name") else {
            return Err(format!("HostSystem {} returned no name property", host.moref));
        };

        for hba in host.array_prop("config.storageDevice.hostBusAdapter") {
            rows.push((
                host.moref.clone(),
                vec![
                    Cell::Text(name.clone()),
                    Cell::opt_text(hba.text_at("device").filter(|s| !s.is_empty())),
                    // The concrete adapter class, which is what distinguishes a
                    // SAS controller from FC or iSCSI.
                    Cell::opt_text(hba.xsi_type.clone()),
                    Cell::opt_text(hba.text_at("status").filter(|s| !s.is_empty())),
                    Cell::opt_text(hba.text_at("bus").filter(|s| !s.is_empty())),
                    Cell::opt_text(hba.text_at("pci").filter(|s| !s.is_empty())),
                    Cell::opt_text(hba.text_at("driver").filter(|s| !s.is_empty())),
                    Cell::opt_text(hba.text_at("model").filter(|s| !s.is_empty())),
                    // Only adapters with a world-wide name carry one; a plain
                    // local SATA/NVMe controller has none, and empty is correct.
                    Cell::opt_text(hba.text_at("nodeWorldWideName").filter(|s| !s.is_empty())),
                ],
            ));
        }
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vHBA",
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

pub async fn fetch_vhba_all(
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
    fn one_row_per_adapter() {
        let snap = captured_snapshot();
        let expected: usize = snap
            .hosts
            .iter()
            .map(|h| h.array_prop("config.storageDevice.hostBusAdapter").len())
            .sum();
        let rows = cells(rows(&snap).expect("named host"));
        assert_eq!(rows.len(), expected);
    }

    /// `Type` is the adapter's concrete class from `xsi:type`, not the element
    /// name, which is the declared array type and identical for every adapter.
    #[test]
    fn type_is_the_concrete_adapter_class() {
        let rows = cells(rows(&captured_snapshot()).expect("named host"));
        if let Some(r) = rows.first() {
            assert!(
                matches!(at(r, "Type"), Cell::Text(ref s) if s.contains("Hba")),
                "expected a Host*Hba class, got {:?}",
                at(r, "Type")
            );
            assert!(matches!(at(r, "Device"), Cell::Text(ref s) if s.starts_with("vmhba")));
        }
    }
}
