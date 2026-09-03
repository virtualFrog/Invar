//! Lookups shared across sheets.
//!
//! Everything here is pure: it reads an `InventorySnapshot` that has already
//! been fetched. Host names used to be their own `retrieve` call; they are now
//! a field on the snapshot, derived from the hosts it already holds.

use crate::vcenter::soap::ManagedObject;
use std::collections::HashMap;

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

/// `VirtualMachine` properties behind `vm_totals_by_host`.
pub const VM_TOTALS_PROPS: &[&str] = &[
    "name",
    "runtime.host",
    "runtime.powerState",
    "config.hardware.numCPU",
    "config.hardware.memoryMB",
];

/// Group VM counts by `HostSystem` moref.
///
/// vCLS VMs are excluded to stay consistent with vInfo and with what the
/// vSphere UI counts.
pub fn vm_totals_by_host(vms: &[ManagedObject]) -> HashMap<String, HostVmTotals> {
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
    totals
}

/// Ratio to two places, or `None` when the denominator is missing or zero.
pub fn ratio(numerator: i64, denominator: Option<i64>) -> Option<f64> {
    match denominator {
        Some(d) if d > 0 => Some((numerator as f64 / d as f64 * 100.0).round() / 100.0),
        _ => None,
    }
}

/// The per-VM columns that RVTools repeats on every VM-derived sheet.
pub struct VmContext {
    pub name: String,
    pub power_state: Option<String>,
    pub template: Option<bool>,
    pub host: Option<String>,
    pub annotation: Option<String>,
}

/// Property paths behind `VmContext`, for callers to concatenate into their own
/// retrieve.
pub const VM_CONTEXT_PROPS: &[&str] = &[
    "name",
    "runtime.powerState",
    "config.template",
    "runtime.host",
    "config.annotation",
];

impl VmContext {
    /// `None` for vCLS VMs, which are vSphere-managed and excluded everywhere.
    pub fn from(
        vm: &ManagedObject,
        hosts: &HashMap<String, String>,
    ) -> Result<Option<Self>, String> {
        let Some(name) = vm.str_prop("name") else {
            return Err(format!("VirtualMachine {} returned no name property", vm.moref));
        };
        if name.starts_with("vCLS-") {
            return Ok(None);
        }
        Ok(Some(Self {
            name,
            power_state: vm.str_prop("runtime.powerState"),
            template: vm.bool_prop("config.template"),
            host: vm
                .str_prop("runtime.host")
                .map(|h| hosts.get(&h).cloned().unwrap_or(h)),
            annotation: vm.str_prop("config.annotation"),
        }))
    }
}

const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;

pub fn bytes_to_mib(bytes: i64) -> f64 {
    (bytes as f64 / BYTES_PER_MIB * 100.0).round() / 100.0
}
