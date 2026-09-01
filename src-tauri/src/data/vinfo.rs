//! vInfo — one row per virtual machine.
//!
//! Every property below was read off the live vCenter before this file was
//! written; see `docs/VCENTER-PROPERTY-REFERENCE.md`.

use super::common::{bytes_to_gib, host_names, percent};
use super::{Cell, Column, Table};
use crate::vcenter::{Session, VCenterConnection};

/// Properties fetched in a single `RetrievePropertiesEx` call.
const PROPS: &[&str] = &[
    "name",
    "config.template",
    "runtime.powerState",
    "runtime.host",
    "guest.ipAddress",
    "guest.hostName",
    "guest.guestFullName",
    "config.guestFullName",
    "guest.toolsVersionStatus",
    "guest.toolsRunningStatus",
    "config.createDate",
    "config.version",
    "config.annotation",
    "config.firmware",
    "config.bootOptions.efiSecureBootEnabled",
    "config.hardware.numCPU",
    "config.hardware.numCoresPerSocket",
    "config.hardware.memoryMB",
    "config.files.vmPathName",
    "config.changeVersion",
    "config.uuid",
    "summary.storage.committed",
    "summary.storage.uncommitted",
    "summary.quickStats.overallCpuUsage",
    "summary.runtime.maxCpuUsage",
    "summary.quickStats.guestMemoryUsage",
    "summary.config.memorySizeMB",
];

/// RVTools' labels, except where our unit differs: RVTools reports
/// `Provisioned MiB` / `In Use MiB`, and we report GiB, so the label says GiB.
/// Never label a GiB value MiB.
pub fn columns() -> Vec<Column> {
    vec![
        Column::text("VM"),
        Column::text("Powerstate"),
        Column::bool("Template"),
        Column::text("DNS Name"),
        Column::number("CPUs"),
        Column::number("Cores p/s"),
        Column::number("Memory"),
        Column::number("CPU Usage (%)"),
        Column::number("Memory Usage (%)"),
        Column::number("Provisioned GiB"),
        Column::number("In Use GiB"),
        Column::text("Primary IP Address"),
        Column::text("OS according to the configuration file"),
        Column::text("OS according to the VMware Tools"),
        Column::text("Tools Version Status"),
        Column::text("Tools Running Status"),
        Column::text("Host"),
        Column::text("Creation date"),
        Column::text("HW version"),
        Column::text("Firmware"),
        Column::bool("EFI Secure boot"),
        Column::text("Path"),
        Column::text("Annotation"),
        Column::text("Change Version"),
        Column::text("VM UUID"),
    ]
}

/// Fetch vInfo rows from one vCenter. Framework-free by design: the Tauri
/// command is a thin wrapper, so a web-server binary can call this untouched.
pub async fn fetch_vinfo_core(session: &Session) -> Result<Vec<Vec<Cell>>, String> {
    let hosts = host_names(session).await?;
    let vms = session.soap.retrieve("VirtualMachine", PROPS).await?;

    let mut rows = Vec::with_capacity(vms.len());
    for vm in vms {
        let Some(name) = vm.str_prop("name") else {
            // Reported rather than dropped: a nameless VM means the query shape
            // is wrong, and silently skipping it would hide that.
            return Err(format!("VirtualMachine {} returned no name property", vm.moref));
        };
        // vCLS VMs are vSphere-managed; vSphere's own VM count excludes them.
        if name.starts_with("vCLS-") {
            continue;
        }

        let committed = vm.i64_prop("summary.storage.committed");
        let uncommitted = vm.i64_prop("summary.storage.uncommitted");
        let provisioned = match (committed, uncommitted) {
            (Some(c), Some(u)) => Some(bytes_to_gib(c + u)),
            _ => None,
        };

        rows.push(vec![
            Cell::Text(name),
            Cell::opt_text(vm.str_prop("runtime.powerState")),
            Cell::opt_bool(vm.bool_prop("config.template")),
            Cell::opt_text(vm.str_prop("guest.hostName")),
            Cell::opt_num(vm.i64_prop("config.hardware.numCPU").map(|v| v as f64)),
            Cell::opt_num(vm.i64_prop("config.hardware.numCoresPerSocket").map(|v| v as f64)),
            Cell::opt_num(vm.i64_prop("config.hardware.memoryMB").map(|v| v as f64)),
            Cell::opt_num(percent(
                vm.i64_prop("summary.quickStats.overallCpuUsage"),
                vm.i64_prop("summary.runtime.maxCpuUsage"),
            )),
            Cell::opt_num(percent(
                vm.i64_prop("summary.quickStats.guestMemoryUsage"),
                vm.i64_prop("summary.config.memorySizeMB"),
            )),
            Cell::opt_num(provisioned),
            Cell::opt_num(committed.map(bytes_to_gib)),
            Cell::opt_text(vm.str_prop("guest.ipAddress")),
            Cell::opt_text(vm.str_prop("config.guestFullName")),
            Cell::opt_text(vm.str_prop("guest.guestFullName")),
            Cell::opt_text(vm.str_prop("guest.toolsVersionStatus")),
            Cell::opt_text(vm.str_prop("guest.toolsRunningStatus")),
            Cell::opt_text(
                vm.str_prop("runtime.host")
                    .map(|h| hosts.get(&h).cloned().unwrap_or(h)),
            ),
            Cell::opt_text(vm.str_prop("config.createDate")),
            Cell::opt_text(vm.str_prop("config.version")),
            Cell::opt_text(vm.str_prop("config.firmware")),
            Cell::opt_bool(vm.bool_prop("config.bootOptions.efiSecureBootEnabled")),
            Cell::opt_text(vm.str_prop("config.files.vmPathName")),
            Cell::opt_text(vm.str_prop("config.annotation")),
            Cell::opt_text(vm.str_prop("config.changeVersion")),
            Cell::opt_text(vm.str_prop("config.uuid")),
        ]);
    }

    Ok(rows)
}

/// Aggregate vInfo across every configured vCenter.
///
/// One unreachable server yields a warning, not an empty table.
pub async fn fetch_vinfo_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    let mut table = Table::new("vInfo", columns()).with_source_column();

    for conn in conns {
        let label = conn.label();
        match cache.get(conn).await {
            Ok(session) => match fetch_vinfo_core(&session).await {
                Ok(rows) => table.extend_from(&label, rows),
                Err(e) => table.warnings.push(format!("{label}: {e}")),
            },
            Err(e) => table.warnings.push(format!("{label}: {e}")),
        }
    }

    table
}
