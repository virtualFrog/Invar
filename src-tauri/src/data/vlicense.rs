//! vLicense — one row per license assigned to the vCenter.
//!
//! `LicenseManager` is a singleton that lives in no container, so a
//! `ContainerView` cannot reach it. It is fetched by its own moref through
//! `SoapClient::retrieve_moref`, which exists for this sheet.
//!
//! `licenses` is a `LicenseManagerLicenseInfo[]`, so its elements carry the
//! type name — verified live.
//!
//! `total = 0` means unlimited rather than none: an evaluation licence reports
//! zero capacity, which is not the same as a licence with no seats left. The
//! reference lab is in evaluation, so that is the case it was checked against.

use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("Name"),
        Column::text("Key"),
        Column::text("Edition"),
        Column::text("Cost Unit"),
        Column::number("Total"),
        Column::number("Used"),
        Column::text("Expiration Date"),
    ]
}

/// A named entry from the licence's `properties` bag.
fn property(lic: &crate::vcenter::xml::Element, key: &str) -> Option<String> {
    lic.children_named("properties")
        .find(|p| p.text_at("key").as_deref() == Some(key))
        .and_then(|p| p.text_at("value"))
        .filter(|s| !s.is_empty())
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let Some(lm) = &snap.license_manager else {
        return Ok(Vec::new());
    };

    let mut rows = Vec::new();
    for lic in lm.array_prop("licenses") {
        let total = lic.text_at("total").and_then(|v| v.parse::<f64>().ok());
        rows.push((
            lm.moref.clone(),
            vec![
                Cell::opt_text(lic.text_at("name").filter(|s| !s.is_empty())),
                Cell::opt_text(lic.text_at("licenseKey").filter(|s| !s.is_empty())),
                Cell::opt_text(lic.text_at("editionKey").filter(|s| !s.is_empty())),
                Cell::opt_text(lic.text_at("costUnit").filter(|s| !s.is_empty())),
                // 0 means unlimited on an evaluation licence, not "no seats".
                Cell::opt_num(total),
                Cell::opt_num(lic.text_at("used").and_then(|v| v.parse::<f64>().ok())),
                // Expiry lives in the properties bag, not as a field.
                Cell::opt_text(property(lic, "expirationDate")),
            ],
        ));
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vLicense",
    columns,
    vm_props: &[],
    host_props: &[],
    dvs_props: &[],
    dvpg_props: &[],
    cluster_props: &[],
    datastore_props: &[],
    rp_props: &[],
    wants_licenses: true,
    wants_about: false,
    wants_files: false,
    source: RowSource::None,
    rows,
};

pub async fn fetch_vlicense_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_licenses, captured_snapshot, cells, col};

    fn snapshot() -> InventorySnapshot {
        captured_snapshot().with_license_manager(Some(captured_licenses()))
    }

    #[test]
    fn one_row_per_license() {
        let rows = cells(rows(&snapshot()).expect("rows build"));
        assert_eq!(rows.len(), 1);
    }

    /// `licenses` is a `LicenseManagerLicenseInfo[]`, so its elements carry the
    /// type name. Reading the field name would give zero rows and no error.
    #[test]
    fn license_fields_come_off_the_typed_array_element() {
        let rows = cells(rows(&snapshot()).expect("rows build"));
        let at = |l: &str| rows[0][col(&columns(), l)].clone();
        assert!(matches!(at("Name"), Cell::Text(ref s) if !s.is_empty()));
        assert!(matches!(at("Key"), Cell::Text(_)));
        assert!(matches!(at("Edition"), Cell::Text(ref s) if s == "eval"));
    }

    /// An evaluation licence reports `total = 0`, meaning unlimited rather than
    /// exhausted. It is passed through as vCenter states it rather than being
    /// reinterpreted into a number RVTools never showed.
    #[test]
    fn an_evaluation_license_reports_zero_total() {
        let rows = cells(rows(&snapshot()).expect("rows build"));
        assert!(matches!(rows[0][col(&columns(), "Total")], Cell::Number(n) if n == 0.0));
    }

    #[test]
    fn no_license_manager_means_no_rows() {
        let rows = cells(rows(&captured_snapshot()).expect("rows build"));
        assert!(rows.is_empty());
    }
}
