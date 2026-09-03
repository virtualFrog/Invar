//! vMultiPath — one row per storage path.
//!
//! The plan sized this as a Phase 5 blocker on the assumption it needed an API
//! area the app could not reach. It does not: `config.storageDevice.multipathInfo`
//! and `config.storageDevice.scsiLun` both come off the `HostSystem` fetch that
//! vHBA and vNIC already do, so this sheet adds no inventory walk.
//!
//! The two properties are shaped differently, and it matters:
//!
//! - `scsiLun` is a **top-level array**, so its elements carry the declared type
//!   name — `<ScsiLun xsi:type="HostScsiDisk">` — not the field name.
//! - `multipathInfo` is a **single object**, so the LUNs beneath it repeat the
//!   *field* name `<lun>`, and each LUN's paths repeat `<path>`.
//!
//! That is the asymmetry `CLAUDE.md` warns about, in one property pair. Reading
//! either the wrong way yields no rows and no error.
//!
//! A row is a *path*, not a device: the sheet exists to show that a LUN is
//! reachable more than one way, and which of those ways is live. The lab's
//! disks are local SAS with a single path each, so every row here has one path
//! and `Working path` is true throughout — correct, and not much of a test of
//! the multi-path case.

use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::soap::ManagedObject;
use crate::vcenter::xml::Element;
use crate::vcenter::VCenterConnection;
use std::collections::HashMap;

/// `HostSystem` properties this sheet reads.
pub const HOST_PROPS: &[&str] = &[
    "name",
    "config.storageDevice.multipathInfo",
    "config.storageDevice.scsiLun",
];

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("Host"),
        Column::text("Device"),
        Column::text("Display Name"),
        Column::text("Policy"),
        Column::text("Runtime Name"),
        Column::text("Path State"),
        Column::text("State"),
        Column::bool("Working path"),
        Column::text("Adapter"),
        Column::text("Transport"),
        Column::text("Vendor"),
        Column::text("Model"),
        Column::text("LUN type"),
        Column::bool("SSD"),
        Column::bool("Local disk"),
        Column::number("Capacity MiB"),
        Column::text("Operational state"),
        Column::number("Queue depth"),
    ]
}

/// `ScsiLun` key → the device, so a path can name what it leads to.
fn luns_by_key<'a>(host: &'a ManagedObject) -> HashMap<String, &'a Element> {
    host.array_prop("config.storageDevice.scsiLun")
        .into_iter()
        .filter_map(|l| Some((l.text_at("key")?, l)))
        .collect()
}

/// `key-vim.host.SerialAttachedHba-vmhba0` → `vmhba0`.
///
/// The adapter arrives as an internal key; the trailing segment is the device
/// name an administrator would recognise, and is what RVTools shows.
fn adapter_name(key: &str) -> Option<String> {
    key.rsplit('-').next().filter(|s| !s.is_empty()).map(str::to_string)
}

/// Capacity in MiB from the block count and block size vCenter reports.
fn capacity_mib(lun: &Element) -> Option<f64> {
    let blocks: i64 = lun.text_at("capacity/block")?.parse().ok()?;
    let size: i64 = lun.text_at("capacity/blockSize")?.parse().ok()?;
    Some(((blocks as f64 * size as f64) / (1024.0 * 1024.0) * 100.0).round() / 100.0)
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let mut rows = Vec::new();

    for host in &snap.hosts {
        let Some(name) = host.str_prop("name") else {
            return Err(format!("HostSystem {} returned no name property", host.moref));
        };
        let luns = luns_by_key(host);

        // `multipathInfo` is one object; its LUNs are `<lun>` children.
        for unit in host.array_prop("config.storageDevice.multipathInfo") {
            let device = unit.text_at("lun").and_then(|k| luns.get(&k).copied());
            let policy = unit.text_at("policy/policy").filter(|s| !s.is_empty());

            for path in unit.children_named("path") {
                let text = |e: Option<&Element>, p: &str| {
                    e.and_then(|x| x.text_at(p)).filter(|s| !s.is_empty())
                };
                rows.push((
                    host.moref.clone(),
                    vec![
                        Cell::Text(name.clone()),
                        Cell::opt_text(text(device, "canonicalName")),
                        Cell::opt_text(text(device, "displayName")),
                        Cell::opt_text(policy.clone()),
                        // vmhba0:C0:T1:L0 — the name esxcli and the UI both use.
                        Cell::opt_text(path.text_at("name").filter(|s| !s.is_empty())),
                        Cell::opt_text(path.text_at("pathState").filter(|s| !s.is_empty())),
                        Cell::opt_text(path.text_at("state").filter(|s| !s.is_empty())),
                        Cell::opt_bool(path.text_at("isWorkingPath").map(|v| v == "true")),
                        Cell::opt_text(path.text_at("adapter").as_deref().and_then(adapter_name)),
                        // The transport's concrete class is what says whether
                        // this is SAS, FC, iSCSI or NVMe.
                        Cell::opt_text(path.child("transport").and_then(|t| t.xsi_type.clone())),
                        Cell::opt_text(text(device, "vendor")),
                        Cell::opt_text(text(device, "model")),
                        Cell::opt_text(text(device, "lunType")),
                        Cell::opt_bool(text(device, "ssd").map(|v| v == "true")),
                        Cell::opt_bool(text(device, "localDisk").map(|v| v == "true")),
                        Cell::opt_num(device.and_then(capacity_mib)),
                        Cell::opt_text(text(device, "operationalState")),
                        Cell::opt_num(
                            text(device, "queueDepth").and_then(|v| v.parse::<f64>().ok()),
                        ),
                    ],
                ));
            }
        }
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vMultiPath",
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
    wants_files: false,
    source: RowSource::Host,
    rows,
};

pub async fn fetch_vmultipath_all(
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

    /// A row is a path, not a device. The captured host has one path per LUN,
    /// so the two counts happen to match here — asserted against the paths so
    /// the test still means something on hardware with real multipathing.
    #[test]
    fn one_row_per_path() {
        let snap = captured_snapshot();
        let expected: usize = snap
            .hosts
            .iter()
            .flat_map(|h| h.array_prop("config.storageDevice.multipathInfo"))
            .map(|u| u.children_named("path").count())
            .sum();
        let rows = cells(rows(&snap).expect("named host"));
        assert_eq!(rows.len(), expected);
        assert!(expected > 0, "the capture should contain storage paths");
    }

    /// The path names its LUN by key, and the device details come from the
    /// separate `scsiLun` array. Joining the two is the whole job of this sheet.
    #[test]
    fn a_path_resolves_the_device_it_leads_to() {
        let rows = cells(rows(&captured_snapshot()).expect("named host"));
        let r = &rows[0];
        assert!(
            matches!(at(r, "Device"), Cell::Text(ref s) if !s.is_empty()),
            "the path should name its device, got {:?}",
            at(r, "Device")
        );
        assert!(matches!(at(r, "Display Name"), Cell::Text(_)));
        assert!(matches!(at(r, "Vendor"), Cell::Text(_)));
        assert!(matches!(at(r, "Capacity MiB"), Cell::Number(n) if n > 0.0));
    }

    /// The adapter arrives as an internal key; the sheet shows the device name.
    #[test]
    fn the_adapter_key_is_reduced_to_its_device_name() {
        assert_eq!(
            adapter_name("key-vim.host.SerialAttachedHba-vmhba0").as_deref(),
            Some("vmhba0")
        );
        assert_eq!(adapter_name("").as_deref(), None);
        let rows = cells(rows(&captured_snapshot()).expect("named host"));
        assert!(
            matches!(at(&rows[0], "Adapter"), Cell::Text(ref s) if s.starts_with("vmhba")),
            "got {:?}",
            at(&rows[0], "Adapter")
        );
    }

    /// `pathState` and the runtime name are what say whether this path is
    /// carrying I/O right now.
    #[test]
    fn path_state_and_runtime_name_come_off_the_path() {
        let rows = cells(rows(&captured_snapshot()).expect("named host"));
        let r = &rows[0];
        assert!(matches!(at(r, "Path State"), Cell::Text(ref s) if !s.is_empty()));
        assert!(
            matches!(at(r, "Runtime Name"), Cell::Text(ref s) if s.contains(':')),
            "expected a vmhbaN:C:T:L runtime name, got {:?}",
            at(r, "Runtime Name")
        );
        assert!(matches!(at(r, "Working path"), Cell::Bool(_)));
    }
}
