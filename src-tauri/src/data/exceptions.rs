//! Exceptions — the VMs that have been special-cased against their cluster.
//!
//! Not an RVTools sheet. A cluster's settings say how it is *meant* to behave;
//! the per-VM overrides say where somebody decided it should not. "What has
//! been special-cased here, and by implication why" is among the first things
//! an engineer needs from an unfamiliar estate, and it is the last thing a
//! flat per-object dump surfaces — the override lives on the cluster object,
//! not on the VM it applies to, so no VM row can show it.
//!
//! One row per override. **An empty sheet means every VM is treated the same,
//! which is the simpler estate to reason about.**
//!
//! ## Two ways to report a VM that has not been overridden at all
//!
//! Both were found in the reference lab, and both would have produced false
//! findings. Verified live 2026-09-10:
//!
//! 1. **A `dasVmConfig` entry is not itself an override.** `vm-19` has one
//!    whose `restartPriority` is `medium` — exactly the cluster default. The
//!    entry exists; the setting does not differ. Only a value that actually
//!    departs from the cluster default is reported.
//! 2. **`clusterIsolationResponse` means "inherit".** It is the value an
//!    isolation-response override takes when it defers to the cluster, so
//!    treating it as a setting would flag every entry in the lab.
//!
//! ## The field that lies
//!
//! `ClusterDasVmConfigInfo` carries `restartPriority` twice: once directly,
//! and once inside `dasSettings`. They disagree. The direct field is the
//! deprecated `DasVmPriority` enum, which has no `highest`, so a VM set to
//! `highest` reports `high` there and `highest` in `dasSettings`. Four of the
//! lab's five real overrides are `highest`, so reading the obvious field would
//! have under-reported every one of them without erroring. `dasSettings` is
//! the authority.
//!
//! ## Deliberately not covered yet
//!
//! `drsVmConfig` (per-VM DRS automation), `rule` (affinity and anti-affinity)
//! and `group` all belong in this register — a DRS rule in particular is a
//! human-authored assertion that two VMs must or must not share a host, which
//! is exactly the kind of intent worth surfacing. The reference lab has **none
//! of them**, so their element shapes could not be read off a live response.
//! Ground rule 1 says a property path is not written from a specification, and
//! a register that silently mis-parses rules is worse than one that says it
//! does not cover them yet.

use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::vcluster::CLUSTER_PROPS;
use super::{Cell, Column, Table};
use crate::vcenter::xml::Element;
use crate::vcenter::VCenterConnection;

/// `VirtualMachine` properties this sheet reads.
///
/// Only enough to name the VM and to let the inventory index resolve its
/// datacenter, cluster and folder: `runtime.host` is how a VM reaches its
/// cluster, since folders and compute are separate branches of the tree.
pub const VM_PROPS: &[&str] = &["name", "runtime.host"];

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("VM"),
        Column::text("Setting"),
        Column::text("This VM"),
        Column::text("Cluster default"),
    ]
}

/// A per-VM HA setting, read from `dasSettings` rather than the deprecated
/// twin on the parent — see the module note.
fn setting(entry: &Element, field: &str) -> Option<String> {
    entry
        .child("dasSettings")
        .and_then(|s| s.text_at(field))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let mut rows = Vec::new();

    for cluster in &snap.clusters {
        let Some(cfg) = cluster.prop("configurationEx") else {
            continue;
        };
        let Some(das) = cfg.child("dasConfig") else {
            continue;
        };
        let defaults = das.child("defaultVmSettings");
        let default_of = |field: &str| {
            defaults
                .and_then(|d| d.text_at(field))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        let default_priority = default_of("restartPriority");
        let default_isolation = default_of("isolationResponse");

        for entry in cfg.children_named("dasVmConfig") {
            let Some(vm_moref) = entry.text_at("key").filter(|s| !s.is_empty()) else {
                continue;
            };
            let vm_name = snap
                .paths
                .name_of(&vm_moref)
                .unwrap_or_else(|| vm_moref.clone());

            let mut push = |label: &str, value: String, default: Option<String>| {
                rows.push((
                    vm_moref.clone(),
                    vec![
                        Cell::Text(vm_name.clone()),
                        Cell::Text(label.to_string()),
                        Cell::Text(value),
                        Cell::opt_text(default),
                    ],
                ));
            };

            // Only a value that actually departs from the cluster default is
            // an override. An entry echoing the default is not a finding.
            if let Some(p) = setting(entry, "restartPriority") {
                if Some(&p) != default_priority.as_ref() {
                    push("HA restart priority", p, default_priority.clone());
                }
            }
            if let Some(i) = setting(entry, "isolationResponse") {
                // The value that means "defer to the cluster".
                if i != "clusterIsolationResponse" && Some(&i) != default_isolation.as_ref() {
                    push("HA isolation response", i, default_isolation.clone());
                }
            }
        }
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "Exceptions",
    columns,
    // VMs are read only to name them and to place them in the inventory tree.
    vm_props: &[VM_PROPS],
    host_props: &[],
    dvs_props: &[],
    dvpg_props: &[],
    // The same `configurationEx` vCluster already fetches whole, so the
    // snapshot unions the two into one retrieve.
    cluster_props: &[CLUSTER_PROPS],
    datastore_props: &[],
    rp_props: &[],
    wants_licenses: false,
    wants_about: false,
    wants_files: false,
    // Each row is about a VM, so Datacenter / Cluster / Folder are appended by
    // the table renderer rather than restated here.
    source: RowSource::Vm,
    rows,
};

pub async fn fetch_exceptions_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{cells, col, vm};
    use crate::vcenter::soap::ManagedObject;
    use crate::vcenter::xml;

    /// A cluster whose `configurationEx` is shaped the way the live response
    /// is: `dasVmConfig` repeats by *field* name inside the parent object, and
    /// each entry carries both the deprecated top-level `restartPriority` and
    /// the authoritative one inside `dasSettings`.
    fn cluster(default_priority: &str, entries: &[(&str, &str, &str, &str)]) -> ManagedObject {
        let vm_entries: String = entries
            .iter()
            .map(|(key, legacy, real, isolation)| {
                format!(
                    "<dasVmConfig><key>{key}</key><restartPriority>{legacy}</restartPriority>\
                     <dasSettings><restartPriority>{real}</restartPriority>\
                     <isolationResponse>{isolation}</isolationResponse></dasSettings></dasVmConfig>"
                )
            })
            .collect();
        let fragment = format!(
            r#"<objects><obj type="ClusterComputeResource">domain-c9</obj><propSet><name>name</name><val>cluster01</val></propSet><propSet><name>configurationEx</name><val><dasConfig><enabled>true</enabled><defaultVmSettings><restartPriority>{default_priority}</restartPriority><isolationResponse>powerOff</isolationResponse></defaultVmSettings></dasConfig>{vm_entries}</val></propSet></objects>"#
        );
        ManagedObject::from_element(&xml::parse(&fragment).expect("fragment parses"))
    }

    fn snapshot(cl: ManagedObject) -> InventorySnapshot {
        InventorySnapshot::from_parts(vec![vm("vm-30", &[("name", "prod-db01")])], Vec::new())
            .with_clusters(vec![cl])
    }

    /// The lab's real shape: `highest` in `dasSettings`, `high` in the
    /// deprecated twin. Reading the twin would report the wrong value.
    #[test]
    fn the_authoritative_priority_comes_from_das_settings() {
        let snap =
            snapshot(cluster("medium", &[("vm-30", "high", "highest", "clusterIsolationResponse")]));
        let cols = columns();
        let rows = cells(rows(&snap).expect("rows"));
        assert_eq!(rows.len(), 1);
        assert!(matches!(&rows[0][col(&cols, "This VM")], Cell::Text(s) if s == "highest"));
        assert!(matches!(&rows[0][col(&cols, "Cluster default")], Cell::Text(s) if s == "medium"));
        assert!(
            matches!(&rows[0][col(&cols, "Setting")], Cell::Text(s) if s == "HA restart priority")
        );
    }

    /// `vm-19` in the lab: an entry exists, but its value is the cluster
    /// default. Having a `dasVmConfig` is not the same as being overridden.
    #[test]
    fn an_entry_matching_the_default_is_not_an_override() {
        let snap = snapshot(cluster(
            "medium",
            &[("vm-19", "medium", "medium", "clusterIsolationResponse")],
        ));
        assert!(rows(&snap).expect("rows").is_empty());
    }

    /// The value that means "defer to the cluster" must not read as a setting,
    /// or every entry in the lab becomes a finding.
    #[test]
    fn cluster_isolation_response_means_inherit() {
        let snap = snapshot(cluster(
            "medium",
            &[("vm-30", "high", "medium", "clusterIsolationResponse")],
        ));
        assert!(
            rows(&snap).expect("rows").is_empty(),
            "priority matches the default and isolation defers to the cluster"
        );
    }

    /// A real isolation override still reports.
    #[test]
    fn a_genuine_isolation_override_is_reported() {
        let snap = snapshot(cluster("medium", &[("vm-30", "medium", "medium", "shutdown")]));
        let cols = columns();
        let rows = cells(rows(&snap).expect("rows"));
        assert_eq!(rows.len(), 1);
        assert!(
            matches!(&rows[0][col(&cols, "Setting")], Cell::Text(s) if s == "HA isolation response")
        );
        assert!(matches!(&rows[0][col(&cols, "This VM")], Cell::Text(s) if s == "shutdown"));
        assert!(
            matches!(&rows[0][col(&cols, "Cluster default")], Cell::Text(s) if s == "powerOff")
        );
    }

    /// The moref is resolved to a name when the VM is in the snapshot.
    #[test]
    fn the_vm_is_named_not_morefed() {
        let snap =
            snapshot(cluster("medium", &[("vm-30", "high", "highest", "clusterIsolationResponse")]));
        let rows = cells(rows(&snap).expect("rows"));
        assert!(matches!(&rows[0][col(&columns(), "VM")], Cell::Text(s) if s == "prod-db01"));
    }
}
