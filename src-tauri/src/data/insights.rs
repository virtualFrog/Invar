//! Dale Insights — the environment rolled up into headline numbers.
//!
//! Built on the same verified fetches the sheets use: topology supplies hosts,
//! clusters and datastores; `vm_totals_by_host` supplies VM, vCPU and vRAM
//! counts. Nothing here queries a property that a sheet has not already proven
//! against the live vCenter.

use super::common::{vm_totals_by_host, VM_TOTALS_PROPS};
use super::topology::{fetch_topology_core, DatastoreNode};
use crate::vcenter::{Session, VCenterConnection};
use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
pub struct StorageByType {
    pub kind: String,
    pub datastores: usize,
    pub capacity_gib: f64,
    pub used_gib: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatastoreUsage {
    pub name: String,
    pub kind: String,
    pub server: String,
    pub capacity_gib: f64,
    pub used_gib: f64,
    pub used_percent: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterSummary {
    pub name: String,
    pub server: String,
    pub hosts: usize,
    pub cores: i64,
    pub dram_gib: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Insights {
    pub servers: Vec<String>,
    pub datacenters: usize,
    pub clusters: usize,
    pub hosts: usize,
    pub hosts_in_maintenance: usize,
    pub hosts_disconnected: usize,

    pub cores: i64,
    /// Physical DRAM, excluding any NVMe memory tier.
    pub dram_gib: f64,
    /// Memory as vCenter reports it — DRAM plus tiers where tiering is on.
    pub memory_total_gib: f64,

    pub vms_total: i64,
    pub vms_powered_on: i64,
    pub vcpus: i64,
    pub vram_gib: f64,

    pub datastores: usize,
    pub storage_capacity_gib: f64,
    pub storage_used_gib: f64,
    pub storage_free_gib: f64,

    /// Derived, but stored rather than computed: the UI consumes this struct as
    /// JSON, where methods do not exist.
    pub storage_used_percent: f64,
    pub vcpu_core_ratio: Option<f64>,

    pub storage_by_type: Vec<StorageByType>,
    /// Fullest datastores first — the ones worth looking at.
    pub top_datastores: Vec<DatastoreUsage>,
    pub cluster_summaries: Vec<ClusterSummary>,

    pub warnings: Vec<String>,
}

impl Insights {
    /// Fill the derived fields once every server has been folded in.
    fn finish(&mut self) {
        self.storage_used_percent = if self.storage_capacity_gib > 0.0 {
            (self.storage_used_gib / self.storage_capacity_gib * 1000.0).round() / 10.0
        } else {
            0.0
        };
        // vCPUs committed per physical core — the classic overcommit ratio.
        self.vcpu_core_ratio = (self.cores > 0)
            .then(|| (self.vcpus as f64 / self.cores as f64 * 100.0).round() / 100.0);
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

async fn accumulate(session: &Session, server: &str, out: &mut Insights) -> Result<(), String> {
    let topology = fetch_topology_core(session, server).await?;
    // The dashboard is not a sheet, so it does its own VM read rather than
    // joining a sheet snapshot. Same property set, so the rollup is identical.
    let vms = session.soap.retrieve("VirtualMachine", VM_TOTALS_PROPS).await?;
    let vm_totals = vm_totals_by_host(&vms);

    out.servers.push(server.to_string());
    out.datacenters += topology.datacenters.len();
    out.clusters += topology.clusters.len();

    for (cluster_name, hosts) in &topology.clusters {
        out.cluster_summaries.push(ClusterSummary {
            name: cluster_name.clone(),
            server: server.to_string(),
            hosts: hosts.len(),
            cores: hosts.iter().filter_map(|h| h.cpu_cores).sum(),
            dram_gib: round2(hosts.iter().filter_map(|h| h.dram_gib).sum()),
        });
    }

    for host in topology.all_hosts() {
        out.hosts += 1;
        if host.in_maintenance {
            out.hosts_in_maintenance += 1;
        } else if host.connection_state.as_deref() != Some("connected") {
            out.hosts_disconnected += 1;
        }
        out.cores += host.cpu_cores.unwrap_or(0);
        // Falls back to reported memory where the host exposes no tier detail.
        out.dram_gib += host.dram_gib.or(host.memory_gib).unwrap_or(0.0);
        out.memory_total_gib += host.memory_gib.unwrap_or(0.0);

        if let Some(t) = vm_totals.get(&host.moref) {
            out.vms_total += t.vms_total;
            out.vms_powered_on += t.vms_powered_on;
            out.vcpus += t.vcpus;
            out.vram_gib += t.vram_mib as f64 / 1024.0;
        }
    }

    for ds in &topology.datastores {
        out.datastores += 1;
        let capacity = ds.capacity_gib.unwrap_or(0.0);
        let used = ds.used_gib().unwrap_or(0.0);
        out.storage_capacity_gib += capacity;
        out.storage_used_gib += used;
        out.storage_free_gib += ds.free_gib.unwrap_or(0.0);

        let kind = ds.kind.clone().unwrap_or_else(|| "Other".into());
        match out.storage_by_type.iter_mut().find(|t| t.kind == kind) {
            Some(entry) => {
                entry.datastores += 1;
                entry.capacity_gib += capacity;
                entry.used_gib += used;
            }
            None => out.storage_by_type.push(StorageByType {
                kind,
                datastores: 1,
                capacity_gib: capacity,
                used_gib: used,
            }),
        }

        out.top_datastores.push(usage(ds, server));
    }

    Ok(())
}

fn usage(ds: &DatastoreNode, server: &str) -> DatastoreUsage {
    DatastoreUsage {
        name: ds.name.clone(),
        kind: ds.kind.clone().unwrap_or_else(|| "Other".into()),
        server: server.to_string(),
        capacity_gib: round2(ds.capacity_gib.unwrap_or(0.0)),
        used_gib: round2(ds.used_gib().unwrap_or(0.0)),
        used_percent: ds.used_percent().unwrap_or(0.0),
    }
}

/// Roll up every configured vCenter. An unreachable server contributes a
/// warning; the totals then describe the servers that did answer, and the UI
/// says so rather than presenting a short total as complete.
pub async fn fetch_insights_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Insights {
    let mut out = Insights::default();

    for conn in conns {
        let label = conn.label();
        match cache.get(conn).await {
            Ok(session) => {
                if let Err(e) = accumulate(&session, &label, &mut out).await {
                    out.warnings.push(format!("{label}: {e}"));
                }
            }
            Err(e) => out.warnings.push(format!("{label}: {e}")),
        }
    }

    out.dram_gib = round2(out.dram_gib);
    out.memory_total_gib = round2(out.memory_total_gib);
    out.vram_gib = round2(out.vram_gib);
    out.storage_capacity_gib = round2(out.storage_capacity_gib);
    out.storage_used_gib = round2(out.storage_used_gib);
    out.storage_free_gib = round2(out.storage_free_gib);
    for t in &mut out.storage_by_type {
        t.capacity_gib = round2(t.capacity_gib);
        t.used_gib = round2(t.used_gib);
    }
    out.storage_by_type.sort_by(|a, b| b.capacity_gib.total_cmp(&a.capacity_gib));
    out.top_datastores.sort_by(|a, b| b.used_percent.total_cmp(&a.used_percent));
    out.top_datastores.truncate(8);
    out.finish();

    out
}
