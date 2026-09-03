//! One inventory fetch, shared by every sheet derived from it.
//!
//! Sheets used to walk the inventory themselves: four of them each ran their
//! own `retrieve("VirtualMachine", ...)`, and `common` added two more walks, so
//! five sheets cost ten full passes, measured live rather than estimated. Ten
//! of RVTools' 27 sheets are
//! VM-derived, so that shape does not survive the parity work in
//! `docs/PARITY-PLAN.md`.
//!
//! Instead a snapshot is fetched once per vCenter, with the union of the
//! requested sheets' property sets, and each sheet is a pure function over it.
//! The second payoff matters as much as the round trips: a sheet that does no
//! I/O is testable against captured XML with no live vCenter.

use super::{Cell, Column, Table};
use crate::vcenter::soap::ManagedObject;
use crate::vcenter::{Session, SessionCache, VCenterConnection};
use std::collections::HashMap;

/// What one vCenter returned for a fetch.
pub struct InventorySnapshot {
    /// The `VI SDK Server` value for rows built from this snapshot.
    pub server: String,
    pub vms: Vec<ManagedObject>,
    pub hosts: Vec<ManagedObject>,
    /// `HostSystem` moref → host name, for resolving `runtime.host`. Derived
    /// from `hosts`, so it costs no extra round trip.
    pub host_names: HashMap<String, String>,
}

impl InventorySnapshot {
    /// Retrieve only the object types the caller asked for. An empty property
    /// set means that type is not queried at all.
    pub async fn fetch(
        session: &Session,
        server: &str,
        vm_props: &[&'static str],
        host_props: &[&'static str],
    ) -> Result<Self, String> {
        // Every VM-derived sheet resolves `runtime.host` to a host name, so a
        // VM fetch always implies at least the hosts' names. This is the walk
        // `common::host_names` used to do on its own.
        let host_props: Vec<&'static str> = if vm_props.is_empty() {
            host_props.to_vec()
        } else {
            union(&[host_props, &["name"]])
        };

        let hosts = if host_props.is_empty() {
            Vec::new()
        } else {
            session.soap.retrieve("HostSystem", &host_props).await?
        };

        let vms = if vm_props.is_empty() {
            Vec::new()
        } else {
            session.soap.retrieve("VirtualMachine", vm_props).await?
        };

        let host_names = hosts
            .iter()
            .filter_map(|h| h.str_prop("name").map(|n| (h.moref.clone(), n)))
            .collect();

        Ok(Self { server: server.to_string(), vms, hosts, host_names })
    }

    /// A snapshot assembled by hand, for tests that have captured XML but no
    /// vCenter to fetch from.
    pub fn from_parts(vms: Vec<ManagedObject>, hosts: Vec<ManagedObject>) -> Self {
        let host_names = hosts
            .iter()
            .filter_map(|h| h.str_prop("name").map(|n| (h.moref.clone(), n)))
            .collect();
        Self { server: "test".into(), vms, hosts, host_names }
    }
}

/// Merge property sets, preserving first-seen order and dropping duplicates.
///
/// Order is kept because vCenter echoes `propSet` back in request order, and a
/// stable request makes captured XML fixtures comparable between runs.
pub fn union(sets: &[&[&'static str]]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for set in sets {
        for prop in *set {
            if !out.contains(prop) {
                out.push(prop);
            }
        }
    }
    out
}

/// Everything the app needs to know about one sheet.
///
/// Adding a sheet is a new module plus one entry in `data::SHEETS`. The UI
/// needs no change: it renders whatever `list_sheets` and `fetch_sheet` return.
pub struct SheetSpec {
    /// RVTools' sheet name, e.g. `vInfo`.
    pub name: &'static str,
    pub columns: fn() -> Vec<Column>,
    /// `VirtualMachine` property sets this sheet reads, unioned at fetch time.
    /// Sets rather than one flat list so a sheet composes shared groups like
    /// `common::VM_CONTEXT_PROPS` instead of copying them. Empty reads nothing.
    pub vm_props: &'static [&'static [&'static str]],
    /// `HostSystem` property sets this sheet reads. Empty reads nothing.
    pub host_props: &'static [&'static [&'static str]],
    /// Pure by design: all I/O happened when the snapshot was built.
    pub rows: fn(&InventorySnapshot) -> Result<Vec<Vec<Cell>>, String>,
}

/// Build every sheet in `specs` from one snapshot per vCenter.
///
/// Never fails as a whole. A vCenter that cannot be reached contributes a
/// warning to every table and no rows, so the healthy servers' data still
/// arrives: for an inventory tool, a short list that looks complete is the
/// worst outcome.
pub async fn fetch_tables(
    specs: &[&SheetSpec],
    conns: &[VCenterConnection],
    cache: &SessionCache,
) -> Vec<Table> {
    let vm_sets: Vec<&[&'static str]> =
        specs.iter().flat_map(|s| s.vm_props.iter().copied()).collect();
    let host_sets: Vec<&[&'static str]> =
        specs.iter().flat_map(|s| s.host_props.iter().copied()).collect();
    let vm_props = union(&vm_sets);
    let host_props = union(&host_sets);

    let mut tables: Vec<Table> = specs
        .iter()
        .map(|s| Table::new(s.name, (s.columns)()).with_source_column())
        .collect();

    for conn in conns {
        let label = conn.label();

        let snapshot = match cache.get(conn).await {
            Ok(session) => {
                InventorySnapshot::fetch(&session, &label, &vm_props, &host_props).await
            }
            Err(e) => Err(e),
        };

        let snapshot = match snapshot {
            Ok(s) => s,
            Err(e) => {
                for table in &mut tables {
                    table.warnings.push(format!("{label}: {e}"));
                }
                continue;
            }
        };

        for (spec, table) in specs.iter().zip(tables.iter_mut()) {
            match (spec.rows)(&snapshot) {
                Ok(rows) => table.extend_from(&label, rows),
                Err(e) => table.warnings.push(format!("{label}: {e}")),
            }
        }
    }

    tables
}

/// One sheet, fetching only the properties that sheet reads.
///
/// The interactive path uses this so opening a single tab does not pay for the
/// whole export's property union.
pub async fn fetch_table(
    spec: &SheetSpec,
    conns: &[VCenterConnection],
    cache: &SessionCache,
) -> Table {
    fetch_tables(&[spec], conns, cache)
        .await
        .pop()
        .expect("one spec yields one table")
}

/// Test-only helpers for building a snapshot out of XML fragments.
///
/// This is the fixture harness Phase 0 of `docs/PARITY-PLAN.md` calls for: a
/// sheet is now a pure function, so it can be exercised with no live vCenter.
/// Real captured `RetrievePropertiesEx` responses drop in as the same shape.
#[cfg(test)]
pub mod test_support {
    use super::*;
    use crate::vcenter::xml;

    /// One `<objects>` entry, shaped the way a real response is.
    pub fn object(moref_type: &str, moref: &str, props: &[(&str, &str)]) -> ManagedObject {
        let props: String = props
            .iter()
            .map(|(name, val)| format!("<propSet><name>{name}</name><val>{val}</val></propSet>"))
            .collect();
        let fragment = format!(
            r#"<objects><obj type="{moref_type}">{moref}</obj>{props}</objects>"#
        );
        ManagedObject::from_element(&xml::parse(&fragment).expect("fragment parses"))
    }

    pub fn vm(moref: &str, props: &[(&str, &str)]) -> ManagedObject {
        object("VirtualMachine", moref, props)
    }

    pub fn host(moref: &str, props: &[(&str, &str)]) -> ManagedObject {
        object("HostSystem", moref, props)
    }

    /// Index of a column by its RVTools label, so tests never hard-code a
    /// position that a new column would silently shift.
    pub fn col(columns: &[Column], label: &str) -> usize {
        columns
            .iter()
            .position(|c| c.label == label)
            .unwrap_or_else(|| panic!("no column labelled {label}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_dedupes_and_keeps_first_seen_order() {
        let merged = union(&[
            &["name", "runtime.host"],
            &["runtime.host", "config.template"],
            &["name"],
        ]);
        assert_eq!(merged, vec!["name", "runtime.host", "config.template"]);
    }

    #[test]
    fn union_of_nothing_is_empty() {
        assert!(union(&[]).is_empty());
        assert!(union(&[&[], &[]]).is_empty());
    }

    #[test]
    fn a_vm_fetch_always_carries_host_names() {
        // Not a fetch (no server here), but the rule it encodes: every
        // VM-derived sheet resolves `runtime.host`, so "name" must survive into
        // the host property set even when no sheet asked for host properties.
        assert_eq!(union(&[&[], &["name"]]), vec!["name"]);
    }

    #[test]
    fn a_snapshot_derives_host_names_from_its_hosts() {
        let snap = InventorySnapshot::from_parts(
            Vec::new(),
            vec![
                test_support::host("host-1", &[("name", "esx1.example.com")]),
                test_support::host("host-2", &[("name", "esx2.example.com")]),
            ],
        );
        assert_eq!(snap.host_names.get("host-1").map(String::as_str), Some("esx1.example.com"));
        assert_eq!(snap.host_names.get("host-2").map(String::as_str), Some("esx2.example.com"));
        assert_eq!(snap.host_names.len(), 2);
    }

    /// The union is what decides how many properties one export asks for. If a
    /// sheet's set stops being merged, the export silently loses columns.
    #[test]
    fn every_registered_sheet_contributes_its_props_to_the_union() {
        let vm_sets: Vec<&[&'static str]> = crate::data::SHEETS
            .iter()
            .flat_map(|s| s.vm_props.iter().copied())
            .collect();
        let merged = union(&vm_sets);

        for spec in crate::data::SHEETS {
            for set in spec.vm_props {
                for prop in *set {
                    assert!(
                        merged.contains(prop),
                        "{} asks for {prop}, which the union dropped",
                        spec.name
                    );
                }
            }
        }
    }
}
