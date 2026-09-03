//! vInfo — one row per virtual machine.
//!
//! Every property below was read off the live vCenter before this file was
//! written; see `docs/VCENTER-PROPERTY-REFERENCE.md`.

use super::common::{bytes_to_gib, percent};
use super::snapshot::{InventorySnapshot, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

/// `VirtualMachine` properties this sheet reads.
pub const VM_PROPS: &[&str] = &[
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

/// Build vInfo rows from an already-fetched snapshot. Framework-free and
/// I/O-free by design: the Tauri command is a thin wrapper, so a web-server
/// binary can call this untouched and a test can call it with captured XML.
pub fn rows(snap: &InventorySnapshot) -> Result<Vec<Vec<Cell>>, String> {
    let hosts = &snap.host_names;

    let mut rows = Vec::with_capacity(snap.vms.len());
    for vm in &snap.vms {
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

pub const SPEC: SheetSpec = SheetSpec {
    name: "vInfo",
    columns,
    vm_props: &[VM_PROPS],
    host_props: &[],
    rows,
};

/// Aggregate vInfo across every configured vCenter.
///
/// One unreachable server yields a warning, not an empty table.
pub async fn fetch_vinfo_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{col, host, vm};

    fn cell(rows: &[Vec<Cell>], row: usize, label: &str) -> Cell {
        rows[row][col(&columns(), label)].clone()
    }

    #[test]
    fn a_vm_resolves_its_host_through_the_snapshot() {
        let snap = InventorySnapshot::from_parts(
            vec![vm("vm-1", &[("name", "OPS91"), ("runtime.host", "host-7")])],
            vec![host("host-7", &[("name", "esx9-01.example.com")])],
        );

        let rows = rows(&snap).expect("named VM yields a row");
        assert_eq!(rows.len(), 1);
        assert!(matches!(cell(&rows, 0, "VM"), Cell::Text(v) if v == "OPS91"));
        assert!(
            matches!(cell(&rows, 0, "Host"), Cell::Text(v) if v == "esx9-01.example.com"),
            "runtime.host should be resolved to the host name"
        );
    }

    /// An unresolvable moref is shown as the moref rather than dropped: losing
    /// the row would under-report the inventory, which is the worst outcome.
    #[test]
    fn an_unknown_host_moref_falls_back_to_the_moref() {
        let snap = InventorySnapshot::from_parts(
            vec![vm("vm-1", &[("name", "OPS91"), ("runtime.host", "host-404")])],
            Vec::new(),
        );
        let rows = rows(&snap).expect("row is still produced");
        assert!(matches!(cell(&rows, 0, "Host"), Cell::Text(v) if v == "host-404"));
    }

    #[test]
    fn vcls_vms_are_excluded() {
        let snap = InventorySnapshot::from_parts(
            vec![
                vm("vm-1", &[("name", "vCLS-4f2e")]),
                vm("vm-2", &[("name", "OPS91")]),
            ],
            Vec::new(),
        );
        let rows = rows(&snap).expect("rows build");
        assert_eq!(rows.len(), 1);
        assert!(matches!(cell(&rows, 0, "VM"), Cell::Text(v) if v == "OPS91"));
    }

    /// A nameless VM means the query shape is wrong. Skipping it silently would
    /// hide that, so it is an error.
    #[test]
    fn a_vm_without_a_name_is_an_error() {
        let snap = InventorySnapshot::from_parts(vec![vm("vm-1", &[])], Vec::new());
        let err = rows(&snap).expect_err("a nameless VM is reported");
        assert!(err.contains("vm-1"), "the error names the object: {err}");
    }

    /// A powered-off VM reports no max CPU. 0% would be a lie, so the cell is
    /// empty instead.
    #[test]
    fn a_missing_denominator_leaves_usage_empty() {
        let snap = InventorySnapshot::from_parts(
            vec![vm(
                "vm-1",
                &[
                    ("name", "OPS91"),
                    ("summary.quickStats.overallCpuUsage", "0"),
                ],
            )],
            Vec::new(),
        );
        let rows = rows(&snap).expect("rows build");
        assert!(matches!(cell(&rows, 0, "CPU Usage (%)"), Cell::Empty));
    }

    /// Provisioned is committed + uncommitted; either one missing means the sum
    /// is unknown, not zero.
    #[test]
    fn provisioned_needs_both_halves() {
        let both = InventorySnapshot::from_parts(
            vec![vm(
                "vm-1",
                &[
                    ("name", "OPS91"),
                    ("summary.storage.committed", "1073741824"),
                    ("summary.storage.uncommitted", "1073741824"),
                ],
            )],
            Vec::new(),
        );
        let built = rows(&both).expect("rows build");
        assert!(matches!(cell(&built, 0, "Provisioned GiB"), Cell::Number(v) if v == 2.0));
        assert!(matches!(cell(&built, 0, "In Use GiB"), Cell::Number(v) if v == 1.0));

        let half = InventorySnapshot::from_parts(
            vec![vm(
                "vm-1",
                &[("name", "OPS91"), ("summary.storage.committed", "1073741824")],
            )],
            Vec::new(),
        );
        let built = rows(&half).expect("rows build");
        assert!(matches!(cell(&built, 0, "Provisioned GiB"), Cell::Empty));
    }
}

/// vInfo over real captured `RetrievePropertiesEx` responses.
#[cfg(test)]
mod captured_tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_snapshot, col};

    fn row_for<'a>(rows: &'a [Vec<Cell>], vm: &str) -> &'a Vec<Cell> {
        let i = col(&columns(), "VM");
        rows.iter()
            .find(|r| matches!(&r[i], Cell::Text(n) if n == vm))
            .unwrap_or_else(|| panic!("no row for {vm}"))
    }

    #[test]
    fn one_row_per_captured_vm() {
        let rows = rows(&captured_snapshot()).expect("captured VMs all have names");
        assert_eq!(rows.len(), 4);
    }

    /// A template is still a vInfo row. RVTools counts templates, which is why
    /// SOAP's 161 rather than REST's 154 is the right VM total for this lab.
    #[test]
    fn a_template_is_a_row_and_is_flagged() {
        let rows = rows(&captured_snapshot()).expect("named VMs");
        let r = row_for(&rows, "Windows Server 2025");
        assert!(matches!(r[col(&columns(), "Template")], Cell::Bool(true)));
        assert!(matches!(&r[col(&columns(), "Powerstate")], Cell::Text(s) if s == "poweredOff"));
    }

    /// `runtime.host` is a moref. Only the captured host resolves to a name;
    /// the other captures reference a host absent from the snapshot, so they
    /// fall back to the raw moref rather than inventing a name or dropping the
    /// row.
    #[test]
    fn host_resolves_when_present_and_falls_back_when_not() {
        let rows = rows(&captured_snapshot()).expect("named VMs");
        let on_host = row_for(&rows, "vSAN File Service Node (1)");
        assert!(
            matches!(&on_host[col(&columns(), "Host")], Cell::Text(h) if h == "esx01.lab.local")
        );
        let elsewhere = row_for(&rows, "appliance01");
        assert!(
            matches!(&elsewhere[col(&columns(), "Host")], Cell::Text(h) if h == "host-28"),
            "an unresolved moref is reported as-is"
        );
    }

    /// A name carrying spaces and parentheses survives the XML round trip.
    #[test]
    fn a_name_with_spaces_and_parentheses_survives() {
        let rows = rows(&captured_snapshot()).expect("named VMs");
        let r = row_for(&rows, "vSAN File Service Node (1)");
        assert!(matches!(&r[col(&columns(), "HW version")], Cell::Text(v) if v == "vmx-21"));
    }

    #[test]
    fn hardware_columns_match_the_capture() {
        let rows = rows(&captured_snapshot()).expect("named VMs");
        let r = row_for(&rows, "appliance01");
        let at = |l: &str| r[col(&columns(), l)].clone();
        assert!(matches!(at("CPUs"), Cell::Number(n) if n == 4.0));
        assert!(matches!(at("Cores p/s"), Cell::Number(n) if n == 1.0));
        assert!(matches!(at("Memory"), Cell::Number(n) if n == 16384.0));
        assert!(matches!(at("Firmware"), Cell::Text(ref s) if s == "efi"));
    }
}
