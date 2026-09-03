//! dvSwitch — one row per distributed virtual switch.
//!
//! The first sheet to need an object type of its own. A `DistributedVirtualSwitch`
//! container view also returns `VmwareDistributedVirtualSwitch`, its subclass,
//! which is what a vSphere DVS actually is — verified live.
//!
//! Rather than guess sub-paths, this fetches the whole `config` and `summary`
//! objects and reads fields off them. That is also what makes the counts cheap:
//! `summary.hostMember` and `summary.vm` are arrays of morefs, so `Host members`
//! and `# VMs` are lengths, not extra queries.

use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

/// `DistributedVirtualSwitch` properties this sheet reads.
pub const DVS_PROPS: &[&str] = &["config", "summary", "overallStatus", "parent"];

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("Switch"),
        Column::text("Datacenter"),
        Column::text("Name"),
        Column::text("Vendor"),
        Column::text("Version"),
        Column::text("Created"),
        Column::number("Host members"),
        Column::number("Max Ports"),
        Column::number("# Ports"),
        Column::number("# VMs"),
        Column::text("CDP Type"),
        Column::text("CDP Operation"),
        Column::number("Max MTU"),
        Column::text("Config status"),
    ]
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let mut rows = Vec::new();

    for dvs in &snap.dvswitches {
        let config = dvs.prop("config");
        let summary = dvs.prop("summary");
        let text = |el: Option<&crate::vcenter::xml::Element>, path: &str| {
            el.and_then(|e| e.text_at(path)).filter(|s| !s.is_empty())
        };
        let num = |el: Option<&crate::vcenter::xml::Element>, path: &str| {
            el.and_then(|e| e.text_at(path)).and_then(|v| v.parse::<f64>().ok())
        };

        // A DVS lives in the datacenter's network folder, so its datacenter is
        // one hop further up than the folder the index already knows.
        let datacenter = dvs
            .moref_prop("parent")
            .and_then(|(parent, _)| snap.paths.datacenter_of(&parent));

        let hosts = summary.map(|s| s.children_named("hostMember").count());
        let vms = summary.map(|s| s.children_named("vm").count());

        rows.push((
            dvs.moref.clone(),
            vec![
                // The switch's UUID, which is how a NIC's backing refers to it.
                Cell::opt_text(text(config, "uuid")),
                Cell::opt_text(datacenter),
                Cell::opt_text(text(config, "name")),
                Cell::opt_text(text(config, "productInfo/vendor")),
                Cell::opt_text(text(config, "productInfo/version")),
                Cell::opt_text(text(config, "createTime")),
                Cell::opt_num(hosts.map(|n| n as f64)),
                Cell::opt_num(num(config, "maxPorts")),
                Cell::opt_num(num(config, "numPorts")),
                Cell::opt_num(vms.map(|n| n as f64)),
                Cell::opt_text(text(config, "linkDiscoveryProtocolConfig/protocol")),
                Cell::opt_text(text(config, "linkDiscoveryProtocolConfig/operation")),
                Cell::opt_num(num(config, "maxMtu")),
                Cell::opt_text(dvs.str_prop("overallStatus")),
            ],
        ));
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "dvSwitch",
    columns,
    vm_props: &[],
    host_props: &[],
    dvs_props: &[DVS_PROPS],
    dvpg_props: &[],
    cluster_props: &[],
    datastore_props: &[],
    rp_props: &[],
    wants_licenses: false,
    wants_about: false,
    // The sheet carries its own Datacenter column, in RVTools' position; it is
    // not a host- or VM-derived sheet, so it gets no generic location columns.
    source: RowSource::None,
    rows,
};

pub async fn fetch_dvswitch_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_dvswitches, captured_snapshot, cells, col};

    fn at(row: &[Cell], label: &str) -> Cell {
        row[col(&columns(), label)].clone()
    }

    fn snapshot() -> InventorySnapshot {
        captured_snapshot().with_distributed(captured_dvswitches(), Vec::new())
    }

    #[test]
    fn one_row_per_switch() {
        let rows = cells(rows(&snapshot()).expect("rows build"));
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn identity_and_product_come_off_config() {
        let rows = cells(rows(&snapshot()).expect("rows build"));
        let r = &rows[0];
        assert!(matches!(at(r, "Vendor"), Cell::Text(ref s) if s.contains("VMware")));
        assert!(matches!(at(r, "Version"), Cell::Text(_)));
        assert!(matches!(at(r, "Name"), Cell::Text(_)));
        // The UUID is how a NIC's distributed-port backing refers to the switch.
        assert!(matches!(at(r, "Switch"), Cell::Text(ref s) if s.len() > 20));
    }

    /// `hostMember` and `vm` are moref arrays on `summary`, so the counts are
    /// lengths rather than extra queries.
    #[test]
    fn member_counts_are_array_lengths() {
        let rows = cells(rows(&snapshot()).expect("rows build"));
        assert!(matches!(at(&rows[0], "Host members"), Cell::Number(n) if n >= 1.0));
        assert!(matches!(at(&rows[0], "# VMs"), Cell::Number(n) if n >= 1.0));
    }

    /// A DVS hangs off the datacenter's network folder, so its datacenter is a
    /// hop beyond the folder — reached through the same index the VM sheets use.
    #[test]
    fn the_switch_names_its_datacenter() {
        let rows = cells(rows(&snapshot()).expect("rows build"));
        assert!(matches!(at(&rows[0], "Datacenter"), Cell::Text(ref s) if s == "datacenter01"));
    }
}
