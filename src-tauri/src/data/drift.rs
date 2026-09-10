//! Cluster Drift — hosts that disagree with the rest of their cluster.
//!
//! Not an RVTools sheet. RVTools, vCheck and AsBuiltReport all report per
//! object: this host has NTP set, that host does not. None of them answers the
//! question an engineer actually asks when handed an unfamiliar vCenter —
//! *is this cluster internally consistent, and if not, where?* A cluster whose
//! hosts differ in EVC mode, ESXi build or CPU model behaves in ways none of
//! its individual rows explain.
//!
//! One row per finding: a host, a setting, its value, and the value the rest
//! of the cluster agrees on. **An empty sheet is the good result.**
//!
//! ## What counts as drift
//!
//! For each cluster and each setting, the majority value is the mode across
//! the hosts that reported one; every host holding a minority value is a row.
//! Ties produce no rows — with two hosts disagreeing one-to-one there is no
//! majority to be the odd one out from, and calling either "wrong" would be
//! invention.
//!
//! ## Why absence is not drift
//!
//! Only hosts that actually reported a value take part, and a setting needs
//! two such hosts before it is compared at all. This is deliberate, and it is
//! the reason the sheet does not flag "host has no NTP configured".
//! `CLAUDE.md` records that vCenter 9.1.0 returned
//! `hardware.systemInfo.serialNumber` for every host and 9.1.1 returned it for
//! none, on the same hardware, with no error. A property that has been retired
//! by an upgrade and a host that genuinely lacks a setting look identical from
//! here. Reporting the difference would turn every such upgrade into a screen
//! of false findings, and a false finding in a consistency report is worse
//! than a missing one — the same reasoning `vhealth.rs` applies to RVTools'
//! `Performance tip` check.
//!
//! Every property read here is already fetched and rendered by `vhost.rs`, so
//! this sheet adds no vCenter round trip and no unverified property path.

use super::common::bytes_to_gib;
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::vhost::HOST_PROPS;
use super::{Cell, Column, Table};
use crate::vcenter::soap::ManagedObject;
use crate::vcenter::VCenterConnection;
use std::collections::BTreeMap;

/// A setting compared across a cluster's hosts.
///
/// `read` returns `None` when the host did not report the setting, which keeps
/// that host out of the comparison entirely rather than treating "absent" as a
/// value of its own.
struct Check {
    /// Shown in the `Setting` column. Plain English rather than a property
    /// path: the reader is an architect, not someone debugging vim25.
    label: &'static str,
    read: fn(&ManagedObject) -> Option<String>,
}

/// Join a repeating property into one comparable string.
///
/// Sorted, so that two hosts listing the same NTP servers in a different order
/// are not reported as differing. Order carries no meaning for any of the
/// settings read here, and flagging it would be noise.
fn joined(h: &ManagedObject, path: &str) -> Option<String> {
    let mut parts: Vec<String> = h
        .array_prop(path)
        .iter()
        .map(|e| e.text.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    parts.sort();
    Some(parts.join(", "))
}

fn text(h: &ManagedObject, path: &str) -> Option<String> {
    h.str_prop(path).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn num(h: &ManagedObject, path: &str) -> Option<String> {
    h.i64_prop(path).map(|n| n.to_string())
}

/// The settings that should match across a cluster, and why each one matters.
///
/// Hardware first (it explains the rest), then the settings that break
/// mobility, then the ones that only bite during an incident.
const CHECKS: &[Check] = &[
    // Mobility. A mismatch here means vMotion between these hosts fails or is
    // refused, which is usually the most consequential thing in the cluster.
    Check { label: "EVC mode", read: |h| text(h, "summary.currentEVCModeKey") },
    Check { label: "CPU model", read: |h| text(h, "summary.hardware.cpuModel") },
    Check { label: "ESXi build", read: |h| text(h, "summary.config.product.build") },
    // Hardware shape. Asymmetric hosts make HA slot sizing and DRS balance
    // behave in ways the cluster's own summary does not explain.
    Check { label: "Hardware vendor", read: |h| text(h, "summary.hardware.vendor") },
    Check { label: "Hardware model", read: |h| text(h, "summary.hardware.model") },
    Check { label: "CPU sockets", read: |h| num(h, "summary.hardware.numCpuPkgs") },
    Check { label: "CPU cores", read: |h| num(h, "summary.hardware.numCpuCores") },
    Check { label: "CPU threads", read: |h| num(h, "summary.hardware.numCpuThreads") },
    // Rounded to whole GiB: two identical hosts can report byte counts that
    // differ slightly, and that is not a finding.
    Check {
        label: "Memory GiB",
        read: |h| {
            h.i64_prop("summary.hardware.memorySize")
                .map(|b| bytes_to_gib(b).round() as i64)
                .map(|g| g.to_string())
        },
    },
    Check { label: "NUMA nodes", read: |h| num(h, "hardware.numaInfo.numNodes") },
    Check { label: "NIC count", read: |h| num(h, "summary.hardware.numNics") },
    Check { label: "HBA count", read: |h| num(h, "summary.hardware.numHBAs") },
    Check { label: "BIOS version", read: |h| text(h, "hardware.biosInfo.biosVersion") },
    // Time. Divergent clocks break Kerberos, certificate validation and every
    // correlation anyone will later try to do across these hosts' logs.
    Check { label: "NTP servers", read: |h| joined(h, "config.dateTimeInfo.ntpConfig.server") },
    Check { label: "Time zone", read: |h| text(h, "config.dateTimeInfo.timeZone.name") },
    // Name resolution.
    Check { label: "DNS servers", read: |h| joined(h, "config.network.dnsConfig.address") },
    Check { label: "DNS domain", read: |h| text(h, "config.network.dnsConfig.domainName") },
    Check {
        label: "DNS search domains",
        read: |h| joined(h, "config.network.dnsConfig.searchDomain"),
    },
    // Posture. One host hardened differently from its peers is either an
    // oversight or an undocumented exception; both are worth surfacing.
    Check { label: "Lockdown mode", read: |h| text(h, "config.lockdownMode") },
    Check {
        label: "vSAN enabled",
        read: |h| h.bool_prop("config.vsanHostConfig.enabled").map(|b| b.to_string()),
    },
    Check {
        label: "Hyperthreading active",
        read: |h| h.bool_prop("config.hyperThread.active").map(|b| b.to_string()),
    },
    Check {
        label: "Power policy",
        read: |h| text(h, "hardware.cpuPowerManagementInfo.currentPolicy"),
    },
];

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("Host"),
        Column::text("Setting"),
        Column::text("This host"),
        Column::text("Cluster majority"),
        Column::number("Hosts agreeing"),
        Column::number("Hosts compared"),
    ]
}

/// The most common value, and how many hosts hold it.
///
/// `None` on a tie: see the module note on why a one-to-one split produces no
/// finding. `BTreeMap` rather than `HashMap` so a tie is broken the same way
/// on every run and the sheet does not reorder itself between exports.
fn majority(values: &[String]) -> Option<(String, usize)> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for v in values {
        *counts.entry(v.as_str()).or_default() += 1;
    }
    let mut best: Vec<(&&str, &usize)> = counts.iter().collect();
    best.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let (top, top_n) = (*best[0].0, *best[0].1);
    if best.len() > 1 && *best[1].1 == top_n {
        return None;
    }
    Some((top.to_string(), top_n))
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    // Group hosts by the cluster they belong to. A standalone host has nothing
    // to be consistent with, so it is not a candidate for drift.
    let mut by_cluster: BTreeMap<String, Vec<&ManagedObject>> = BTreeMap::new();
    for h in &snap.hosts {
        if let Some(cluster) = snap.paths.cluster_of_host(&h.moref) {
            by_cluster.entry(cluster).or_default().push(h);
        }
    }

    let mut rows = Vec::new();
    for (_cluster, hosts) in by_cluster {
        if hosts.len() < 2 {
            continue;
        }
        for check in CHECKS {
            // Only hosts that reported the setting take part.
            let reported: Vec<(&ManagedObject, String)> =
                hosts.iter().filter_map(|h| (check.read)(h).map(|v| (*h, v))).collect();
            if reported.len() < 2 {
                continue;
            }
            let values: Vec<String> = reported.iter().map(|(_, v)| v.clone()).collect();
            let Some((winner, agreeing)) = majority(&values) else {
                continue;
            };
            for (host, value) in &reported {
                if *value == winner {
                    continue;
                }
                let name = host.str_prop("name").unwrap_or_else(|| host.moref.clone());
                rows.push((
                    host.moref.clone(),
                    vec![
                        Cell::Text(name),
                        Cell::Text(check.label.to_string()),
                        Cell::Text(value.clone()),
                        Cell::Text(winner.clone()),
                        Cell::Number(agreeing as f64),
                        Cell::Number(reported.len() as f64),
                    ],
                ));
            }
        }
    }
    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "Cluster Drift",
    columns,
    vm_props: &[],
    // The same set vHost already fetches, so this sheet costs no round trip of
    // its own — the snapshot unions the two into one retrieve.
    host_props: &[HOST_PROPS],
    dvs_props: &[],
    dvpg_props: &[],
    cluster_props: &[],
    datastore_props: &[],
    rp_props: &[],
    wants_licenses: false,
    wants_about: false,
    wants_files: false,
    // Each row describes a host, so Datacenter and Cluster are appended by the
    // table renderer rather than restated as columns here.
    source: RowSource::Host,
    rows,
};

pub async fn fetch_drift_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_many, cells, col, CONTAINERS};
    use crate::vcenter::xml;

    /// A host hanging off `domain-c9`, the cluster in the `CONTAINERS` capture.
    ///
    /// `parent` needs its `type` attribute: `PathIndex` reads the declared
    /// managed-object type off the attribute, never off the moref prefix, so a
    /// bare `<val>` would leave the host clusterless and silently untested.
    /// `props` values are inserted as the *content* of `<val>`, so a repeating
    /// property is written as its child elements.
    fn host_in_cluster(moref: &str, name: &str, props: &[(&str, &str)]) -> ManagedObject {
        let extra: String = props
            .iter()
            .map(|(n, v)| format!("<propSet><name>{n}</name><val>{v}</val></propSet>"))
            .collect();
        let fragment = format!(
            r#"<objects xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><obj type="HostSystem">{moref}</obj><propSet><name>name</name><val>{name}</val></propSet><propSet><name>parent</name><val type="ClusterComputeResource" xsi:type="ManagedObjectReference">domain-c9</val></propSet>{extra}</objects>"#
        );
        ManagedObject::from_element(&xml::parse(&fragment).expect("fragment parses"))
    }

    fn snapshot(hosts: Vec<ManagedObject>) -> InventorySnapshot {
        InventorySnapshot::from_parts_with_containers(
            Vec::new(),
            hosts,
            captured_many(CONTAINERS),
        )
    }

    fn evc(moref: &str, name: &str, mode: &str) -> ManagedObject {
        host_in_cluster(moref, name, &[("summary.currentEVCModeKey", mode)])
    }

    #[test]
    fn a_uniform_cluster_reports_nothing() {
        let snap = snapshot(vec![
            evc("host-1", "esx01", "intel-broadwell"),
            evc("host-2", "esx02", "intel-broadwell"),
            evc("host-3", "esx03", "intel-broadwell"),
        ]);
        assert!(rows(&snap).expect("rows").is_empty(), "a consistent cluster is not a finding");
    }

    #[test]
    fn the_minority_host_is_the_one_reported() {
        let snap = snapshot(vec![
            evc("host-1", "esx01", "intel-broadwell"),
            evc("host-2", "esx02", "intel-broadwell"),
            evc("host-3", "esx03", "intel-skylake"),
        ]);
        let cols = columns();
        let rows = cells(rows(&snap).expect("rows"));
        assert_eq!(rows.len(), 1, "only the odd host out is a row");
        let r = &rows[0];
        assert!(matches!(&r[col(&cols, "Host")], Cell::Text(s) if s == "esx03"));
        assert!(matches!(&r[col(&cols, "Setting")], Cell::Text(s) if s == "EVC mode"));
        assert!(matches!(&r[col(&cols, "This host")], Cell::Text(s) if s == "intel-skylake"));
        assert!(
            matches!(&r[col(&cols, "Cluster majority")], Cell::Text(s) if s == "intel-broadwell")
        );
        assert!(matches!(r[col(&cols, "Hosts agreeing")], Cell::Number(n) if n == 2.0));
        assert!(matches!(r[col(&cols, "Hosts compared")], Cell::Number(n) if n == 3.0));
    }

    /// With two hosts disagreeing one-to-one there is no majority, so naming
    /// either as the deviant would be invention.
    #[test]
    fn an_even_split_has_no_odd_one_out() {
        let snap = snapshot(vec![
            evc("host-1", "esx01", "intel-broadwell"),
            evc("host-2", "esx02", "intel-skylake"),
        ]);
        assert!(rows(&snap).expect("rows").is_empty());
    }

    /// The `hardware.systemInfo.serialNumber` case from `CLAUDE.md`: an upgrade
    /// retires a property and every host stops reporting it. That must read as
    /// "nothing to compare", never as estate-wide drift.
    #[test]
    fn a_property_no_host_reports_is_not_drift() {
        let snap = snapshot(vec![
            host_in_cluster("host-1", "esx01", &[]),
            host_in_cluster("host-2", "esx02", &[]),
            host_in_cluster("host-3", "esx03", &[]),
        ]);
        assert!(rows(&snap).expect("rows").is_empty());
    }

    /// One reporting host is not a majority of anything.
    #[test]
    fn a_property_only_one_host_reports_is_not_drift() {
        let snap = snapshot(vec![
            evc("host-1", "esx01", "intel-broadwell"),
            host_in_cluster("host-2", "esx02", &[]),
            host_in_cluster("host-3", "esx03", &[]),
        ]);
        assert!(rows(&snap).expect("rows").is_empty());
    }

    /// A host outside a cluster has no peers to be consistent with.
    #[test]
    fn standalone_hosts_are_not_compared() {
        let lone = crate::data::snapshot::test_support::host(
            "host-9",
            &[("name", "esx09"), ("summary.currentEVCModeKey", "intel-skylake")],
        );
        let snap = snapshot(vec![
            evc("host-1", "esx01", "intel-broadwell"),
            evc("host-2", "esx02", "intel-broadwell"),
            lone,
        ]);
        assert!(rows(&snap).expect("rows").is_empty());
    }

    /// Same servers, different order. Order carries no meaning here, and
    /// reporting it would bury the real findings in noise.
    #[test]
    fn repeating_values_compare_order_insensitively() {
        let ntp = |m: &str, n: &str, a: &str, b: &str| {
            host_in_cluster(
                m,
                n,
                &[(
                    "config.dateTimeInfo.ntpConfig.server",
                    &format!("<string>{a}</string><string>{b}</string>"),
                )],
            )
        };
        let snap = snapshot(vec![
            ntp("host-1", "esx01", "10.0.0.1", "10.0.0.2"),
            ntp("host-2", "esx02", "10.0.0.2", "10.0.0.1"),
            ntp("host-3", "esx03", "10.0.0.1", "10.0.0.2"),
        ]);
        assert!(rows(&snap).expect("rows").is_empty());

        // But a genuinely different server still is a finding.
        let snap = snapshot(vec![
            ntp("host-1", "esx01", "10.0.0.1", "10.0.0.2"),
            ntp("host-2", "esx02", "10.0.0.1", "10.0.0.2"),
            ntp("host-3", "esx03", "10.0.0.1", "10.0.0.9"),
        ]);
        let rows = cells(rows(&snap).expect("rows"));
        assert_eq!(rows.len(), 1);
        assert!(matches!(&rows[0][col(&columns(), "Setting")], Cell::Text(s) if s == "NTP servers"));
    }

    /// Byte counts that differ trivially are the same host spec, so memory is
    /// compared in whole GiB.
    #[test]
    fn memory_is_compared_in_whole_gib() {
        let mem = |m: &str, n: &str, bytes: &str| {
            host_in_cluster(m, n, &[("summary.hardware.memorySize", bytes)])
        };
        let snap = snapshot(vec![
            mem("host-1", "esx01", "274877906944"),
            mem("host-2", "esx02", "274877906944"),
            // ~2 MiB less: the same 256 GiB host.
            mem("host-3", "esx03", "274875809792"),
        ]);
        assert!(rows(&snap).expect("rows").is_empty(), "trivial byte differences are not drift");
    }

    /// Several settings differing on one host produce one row each, so the
    /// sheet reads as a list of findings rather than a wide diff.
    #[test]
    fn each_differing_setting_is_its_own_row() {
        let full = |m: &str, n: &str, build: &str, bios: &str| {
            host_in_cluster(
                m,
                n,
                &[("summary.config.product.build", build), ("hardware.biosInfo.biosVersion", bios)],
            )
        };
        let snap = snapshot(vec![
            full("host-1", "esx01", "24022510", "U30"),
            full("host-2", "esx02", "24022510", "U30"),
            full("host-3", "esx03", "23825572", "U46"),
        ]);
        let cols = columns();
        let rows = cells(rows(&snap).expect("rows"));
        assert_eq!(rows.len(), 2);
        let settings: Vec<String> = rows
            .iter()
            .map(|r| match &r[col(&cols, "Setting")] {
                Cell::Text(s) => s.clone(),
                other => panic!("expected text, got {other:?}"),
            })
            .collect();
        assert!(settings.contains(&"ESXi build".to_string()));
        assert!(settings.contains(&"BIOS version".to_string()));
        assert!(rows.iter().all(|r| matches!(&r[col(&cols, "Host")], Cell::Text(s) if s == "esx03")));
    }
}
