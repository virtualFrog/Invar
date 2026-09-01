//! Lookups shared across sheets.

use crate::vcenter::Session;
use std::collections::HashMap;

/// `HostSystem` moref → host name, for resolving `runtime.host` references.
pub async fn host_names(session: &Session) -> Result<HashMap<String, String>, String> {
    let hosts = session.soap.retrieve("HostSystem", &["name"]).await?;
    Ok(hosts
        .into_iter()
        .filter_map(|h| h.str_prop("name").map(|n| (h.moref, n)))
        .collect())
}

const BYTES_PER_GIB: f64 = 1024.0 * 1024.0 * 1024.0;

pub fn bytes_to_gib(bytes: i64) -> f64 {
    (bytes as f64 / BYTES_PER_GIB * 100.0).round() / 100.0
}

/// Percentage, rounded to two places. `None` when the denominator is missing or
/// zero — a powered-off VM reports no max CPU, and 0% would be a lie.
pub fn percent(used: Option<i64>, total: Option<i64>) -> Option<f64> {
    match (used, total) {
        (Some(u), Some(t)) if t > 0 => Some((u as f64 / t as f64 * 10000.0).round() / 100.0),
        _ => None,
    }
}

/// Per-host rollup of the VMs registered on it, for vHost's VM columns.
#[derive(Debug, Default, Clone)]
pub struct HostVmTotals {
    pub vms_total: i64,
    pub vms_powered_on: i64,
    pub vcpus: i64,
    pub vram_mib: i64,
}

/// Group VM counts by `HostSystem` moref.
///
/// vCLS VMs are excluded to stay consistent with vInfo and with what the
/// vSphere UI counts.
pub async fn vm_totals_by_host(
    session: &crate::vcenter::Session,
) -> Result<HashMap<String, HostVmTotals>, String> {
    let vms = session
        .soap
        .retrieve(
            "VirtualMachine",
            &[
                "name",
                "runtime.host",
                "runtime.powerState",
                "config.hardware.numCPU",
                "config.hardware.memoryMB",
            ],
        )
        .await?;

    let mut totals: HashMap<String, HostVmTotals> = HashMap::new();
    for vm in vms {
        if vm.str_prop("name").is_some_and(|n| n.starts_with("vCLS-")) {
            continue;
        }
        let Some(host) = vm.str_prop("runtime.host") else {
            continue; // an unregistered VM belongs to no host's totals
        };
        let entry = totals.entry(host).or_default();
        entry.vms_total += 1;
        if vm.str_prop("runtime.powerState").as_deref() == Some("poweredOn") {
            entry.vms_powered_on += 1;
        }
        entry.vcpus += vm.i64_prop("config.hardware.numCPU").unwrap_or(0);
        entry.vram_mib += vm.i64_prop("config.hardware.memoryMB").unwrap_or(0);
    }
    Ok(totals)
}

/// Ratio to two places, or `None` when the denominator is missing or zero.
pub fn ratio(numerator: i64, denominator: Option<i64>) -> Option<f64> {
    match denominator {
        Some(d) if d > 0 => Some((numerator as f64 / d as f64 * 100.0).round() / 100.0),
        _ => None,
    }
}
