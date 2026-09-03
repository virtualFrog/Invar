//! vSource — one row per connected vCenter, describing the vCenter itself.
//!
//! Comes from `ServiceContent.about`, not from a ContainerView: the vCenter is
//! not an object in its own inventory. `RetrieveServiceContent` also needs no
//! authentication, which is why `test_connection` already uses it.
//!
//! One row per configured connection, which is the whole point of the sheet in
//! a multi-vCenter export.

use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("Name"),
        Column::text("OS type"),
        Column::text("API type"),
        Column::text("API version"),
        Column::text("Version"),
        Column::text("Patch level"),
        Column::text("Build"),
        Column::text("Fullname"),
        Column::text("Product name"),
        Column::text("Product version"),
        Column::text("Product line"),
        Column::text("Vendor"),
    ]
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let Some(about) = &snap.about else {
        return Ok(Vec::new());
    };
    let t = |field: &str| about.text_at(field).filter(|s| !s.is_empty());

    Ok(vec![(
        // `about` is not a managed object, so there is no moref to carry. The
        // instance UUID is the closest stable identity vCenter offers.
        t("instanceUuid").unwrap_or_default(),
        vec![
            Cell::opt_text(t("name")),
            Cell::opt_text(t("osType")),
            Cell::opt_text(t("apiType")),
            Cell::opt_text(t("apiVersion")),
            Cell::opt_text(t("version")),
            Cell::opt_text(t("patchLevel")),
            Cell::opt_text(t("build")),
            Cell::opt_text(t("fullName")),
            Cell::opt_text(t("licenseProductName")),
            Cell::opt_text(t("licenseProductVersion")),
            Cell::opt_text(t("productLineId")),
            Cell::opt_text(t("vendor")),
        ],
    )])
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vSource",
    columns,
    vm_props: &[],
    host_props: &[],
    dvs_props: &[],
    dvpg_props: &[],
    cluster_props: &[],
    datastore_props: &[],
    rp_props: &[],
    wants_licenses: false,
    wants_about: true,
    source: RowSource::None,
    rows,
};

pub async fn fetch_vsource_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_about, captured_snapshot, cells, col};

    fn snapshot() -> InventorySnapshot {
        captured_snapshot().with_about(captured_about())
    }

    #[test]
    fn one_row_per_vcenter() {
        let rows = cells(rows(&snapshot()).expect("rows build"));
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn version_fields_come_from_service_content() {
        let rows = cells(rows(&snapshot()).expect("rows build"));
        let at = |l: &str| rows[0][col(&columns(), l)].clone();
        assert!(matches!(at("Name"), Cell::Text(ref s) if s.contains("vCenter")));
        assert!(matches!(at("Vendor"), Cell::Text(ref s) if s.contains("VMware")));
        assert!(matches!(at("API type"), Cell::Text(ref s) if s == "VirtualCenter"));
        assert!(matches!(at("Build"), Cell::Text(_)));
        assert!(matches!(at("OS type"), Cell::Text(ref s) if s.starts_with("linux")));
    }

    /// A vCenter that was never queried yields no row rather than a row of
    /// blanks, which would read as a vCenter reporting nothing about itself.
    #[test]
    fn no_service_content_means_no_row() {
        let rows = cells(rows(&captured_snapshot()).expect("rows build"));
        assert!(rows.is_empty());
    }
}
