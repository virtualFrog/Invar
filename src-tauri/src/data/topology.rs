//! Host and storage topology: which hosts sit in which cluster, and which
//! datastores each host has mounted.
//!
//! This is the one view that is a graph rather than a table, so it has its own
//! shape instead of `Table`. Property paths were verified against the live
//! vCenter first; `Datastore.host` is a `DatastoreHostMount[]` whose entries
//! carry the host moref in `<key type="HostSystem">`.

use super::common::bytes_to_gib;
use crate::vcenter::{Session, VCenterConnection};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct HostNode {
    pub moref: String,
    pub name: String,
    pub cluster: Option<String>,
    pub connection_state: Option<String>,
    pub in_maintenance: bool,
    pub cpu_cores: Option<i64>,
    /// Total memory as vCenter reports it. With memory tiering enabled this is
    /// DRAM *plus* the NVMe tier, so it is shown alongside `dram_gib` rather
    /// than on its own — see the vHost sheet for the same treatment.
    pub memory_gib: Option<f64>,
    pub dram_gib: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatastoreNode {
    pub moref: String,
    pub name: String,
    /// `VMFS`, `NFS`, `vsan`, …
    pub kind: Option<String>,
    pub capacity_gib: Option<f64>,
    pub free_gib: Option<f64>,
    pub vm_count: usize,
    /// Morefs of the hosts that have this datastore mounted.
    pub mounted_by: Vec<String>,
    pub accessible: Option<bool>,
}

impl DatastoreNode {
    pub fn used_gib(&self) -> Option<f64> {
        match (self.capacity_gib, self.free_gib) {
            (Some(c), Some(f)) => Some(((c - f) * 100.0).round() / 100.0),
            _ => None,
        }
    }

    pub fn used_percent(&self) -> Option<f64> {
        match (self.capacity_gib, self.used_gib()) {
            (Some(c), Some(u)) if c > 0.0 => Some((u / c * 1000.0).round() / 10.0),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerTopology {
    /// The vCenter these objects came from — RVTools' `VI SDK Server`.
    pub server: String,
    pub datacenters: Vec<String>,
    /// Cluster name → hosts, in the order vCenter returned them.
    pub clusters: Vec<(String, Vec<HostNode>)>,
    /// Hosts that belong to no cluster.
    pub standalone_hosts: Vec<HostNode>,
    pub datastores: Vec<DatastoreNode>,
}

impl ServerTopology {
    pub fn all_hosts(&self) -> Vec<&HostNode> {
        self.clusters
            .iter()
            .flat_map(|(_, hosts)| hosts.iter())
            .chain(self.standalone_hosts.iter())
            .collect()
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Topology {
    pub servers: Vec<ServerTopology>,
    pub warnings: Vec<String>,
}

const HOST_PROPS: &[&str] = &[
    "name",
    "runtime.connectionState",
    "runtime.inMaintenanceMode",
    "summary.hardware.numCpuCores",
    "summary.hardware.memorySize",
    "hardware.memoryTierInfo",
];

const DATASTORE_PROPS: &[&str] = &[
    "name",
    "summary.type",
    "summary.capacity",
    "summary.freeSpace",
    "summary.accessible",
    "host",
    "vm",
];

pub async fn fetch_topology_core(session: &Session, server: &str) -> Result<ServerTopology, String> {
    let datacenters = session
        .soap
        .retrieve("Datacenter", &["name"])
        .await?
        .into_iter()
        .filter_map(|d| d.str_prop("name"))
        .collect();

    let hosts_raw = session.soap.retrieve("HostSystem", HOST_PROPS).await?;
    let mut hosts: Vec<HostNode> = Vec::with_capacity(hosts_raw.len());
    for h in hosts_raw {
        let Some(name) = h.str_prop("name") else {
            return Err(format!("HostSystem {} returned no name property", h.moref));
        };
        hosts.push(HostNode {
            moref: h.moref.clone(),
            name,
            cluster: None,
            connection_state: h.str_prop("runtime.connectionState"),
            in_maintenance: h.bool_prop("runtime.inMaintenanceMode").unwrap_or(false),
            cpu_cores: h.i64_prop("summary.hardware.numCpuCores"),
            memory_gib: h.i64_prop("summary.hardware.memorySize").map(bytes_to_gib),
            dram_gib: h
                .array_prop("hardware.memoryTierInfo")
                .iter()
                .find(|t| t.text_at("type").as_deref() == Some("DRAM"))
                .and_then(|t| t.text_at("size"))
                .and_then(|s| s.parse::<i64>().ok())
                .map(bytes_to_gib),
        });
    }

    // Cluster membership comes from the cluster's own host list rather than each
    // host's `parent`, which points at a ComputeResource wrapper for standalone
    // hosts and needs a second lookup to name.
    let clusters_raw = session
        .soap
        .retrieve("ClusterComputeResource", &["name", "host"])
        .await?;

    let mut clusters: Vec<(String, Vec<HostNode>)> = Vec::new();
    for c in clusters_raw {
        let Some(cluster_name) = c.str_prop("name") else {
            return Err(format!("ClusterComputeResource {} returned no name", c.moref));
        };
        let members: Vec<String> = c
            .array_prop("host")
            .iter()
            .map(|m| m.text.clone())
            .filter(|m| !m.is_empty())
            .collect();

        let mut members_resolved = Vec::new();
        for moref in members {
            if let Some(host) = hosts.iter_mut().find(|h| h.moref == moref) {
                host.cluster = Some(cluster_name.clone());
                members_resolved.push(host.clone());
            }
        }
        clusters.push((cluster_name, members_resolved));
    }

    let standalone_hosts: Vec<HostNode> =
        hosts.iter().filter(|h| h.cluster.is_none()).cloned().collect();

    let datastores_raw = session.soap.retrieve("Datastore", DATASTORE_PROPS).await?;
    let mut datastores = Vec::with_capacity(datastores_raw.len());
    for d in datastores_raw {
        let Some(name) = d.str_prop("name") else {
            return Err(format!("Datastore {} returned no name property", d.moref));
        };
        // Each mount is a <DatastoreHostMount> whose <key type="HostSystem">
        // names the host.
        let mounted_by = d
            .array_prop("host")
            .iter()
            .filter_map(|m| m.text_at("key"))
            .filter(|m| !m.is_empty())
            .collect();

        datastores.push(DatastoreNode {
            moref: d.moref.clone(),
            name,
            kind: d.str_prop("summary.type"),
            capacity_gib: d.i64_prop("summary.capacity").map(bytes_to_gib),
            free_gib: d.i64_prop("summary.freeSpace").map(bytes_to_gib),
            vm_count: d.array_prop("vm").len(),
            mounted_by,
            accessible: d.bool_prop("summary.accessible"),
        });
    }
    datastores.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(ServerTopology {
        server: server.to_string(),
        datacenters,
        clusters,
        standalone_hosts,
        datastores,
    })
}

/// Topology across every configured vCenter. One unreachable server yields a
/// warning, not an empty report.
pub async fn fetch_topology_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Topology {
    let mut topology = Topology::default();
    for conn in conns {
        let label = conn.label();
        match cache.get(conn).await {
            Ok(session) => match fetch_topology_core(&session, &label).await {
                Ok(t) => topology.servers.push(t),
                Err(e) => topology.warnings.push(format!("{label}: {e}")),
            },
            Err(e) => topology.warnings.push(format!("{label}: {e}")),
        }
    }
    topology
}
