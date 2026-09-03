//! vHost — one row per ESXi host.
//!
//! A single `HostSystem` query also carries the arrays behind vNIC, vHBA,
//! vSwitch, vPort and vSC_VMK, so those sheets can reuse `PROPS` rather than
//! re-querying. Every path below was read off the live vCenter first.

use super::common::{bytes_to_gib, ratio, vm_totals_by_host, HostVmTotals, VM_TOTALS_PROPS};
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::soap::ManagedObject;
use crate::vcenter::VCenterConnection;

/// `HostSystem` properties this sheet reads.
///
/// The same query also carries the arrays behind vNIC, vHBA, vSwitch, vPort and
/// vSC_VMK, so those sheets can declare these props and share this one fetch.
pub const HOST_PROPS: &[&str] = &[
    "name",
    "overallStatus",
    "runtime.connectionState",
    "runtime.powerState",
    "runtime.inMaintenanceMode",
    "runtime.bootTime",
    "summary.hardware.vendor",
    "summary.hardware.model",
    "summary.hardware.cpuModel",
    "summary.hardware.cpuMhz",
    "summary.hardware.numCpuPkgs",
    "summary.hardware.numCpuCores",
    "summary.hardware.numCpuThreads",
    "summary.hardware.memorySize",
    "hardware.memoryTierInfo",
    "summary.hardware.numNics",
    "summary.hardware.numHBAs",
    "summary.hardware.uuid",
    "summary.quickStats.overallCpuUsage",
    "summary.quickStats.overallMemoryUsage",
    "summary.currentEVCModeKey",
    "summary.maxEVCModeKey",
    "capability.vmotionSupported",
    "capability.storageVMotionSupported",
    "config.product.fullName",
    "config.hyperThread.available",
    "config.hyperThread.active",
    "config.network.dnsConfig.domainName",
    "config.network.dnsConfig.address",
    "config.network.dnsConfig.searchDomain",
    "config.network.dnsConfig.dhcp",
    "config.dateTimeInfo.timeZone.name",
    "config.dateTimeInfo.timeZone.gmtOffset",
    "config.dateTimeInfo.ntpConfig.server",
    "config.service.service",
    "hardware.biosInfo.biosVersion",
    "hardware.biosInfo.releaseDate",
    "hardware.cpuPowerManagementInfo.currentPolicy",
    "hardware.systemInfo.serialNumber",
    "hardware.systemInfo.otherIdentifyingInfo",
];

/// RVTools' labels, except where our unit differs: RVTools reports `# Memory`
/// and `vRAM` in MiB and we report GiB, so those labels say GiB. `Connection
/// state` and `Power state` are ours — RVTools' vHost has no equivalent.
pub fn columns() -> Vec<Column> {
    vec![
        Column::text("Host"),
        Column::text("Config status"),
        Column::bool("in Maintenance Mode"),
        Column::text("Connection state"),
        Column::text("Power state"),
        Column::text("CPU Model"),
        Column::number("Speed"),
        Column::bool("HT Available"),
        Column::bool("HT Active"),
        Column::number("# CPU"),
        Column::number("Cores per CPU"),
        Column::number("# Cores"),
        Column::number("# CPU Threads"),
        Column::number("CPU usage %"),
        Column::number("# Memory GiB"),
        Column::text("Memory Tiering Type"),
        Column::number("DRAM GiB"),
        Column::number("NVMe Tier GiB"),
        Column::number("Memory usage %"),
        Column::number("# NICs"),
        Column::number("# HBAs"),
        Column::number("# VMs total"),
        Column::number("# VMs"),
        Column::number("VMs per Core"),
        Column::number("# vCPUs"),
        Column::number("vCPUs per Core"),
        Column::number("vRAM GiB"),
        Column::bool("VMotion support"),
        Column::bool("Storage VMotion support"),
        Column::text("Current EVC"),
        Column::text("Max EVC"),
        Column::text("Current CPU power man. policy"),
        Column::text("ESX Version"),
        Column::text("Boot time"),
        Column::text("DNS Servers"),
        Column::bool("DHCP"),
        Column::text("Domain"),
        Column::text("DNS Search Order"),
        Column::text("NTP Server(s)"),
        Column::bool("NTPD running"),
        Column::text("Time Zone"),
        Column::number("GMT Offset"),
        Column::text("Vendor"),
        Column::text("Model"),
        Column::text("Serial number"),
        Column::text("Service tag"),
        Column::text("BIOS Version"),
        Column::text("BIOS Date"),
        Column::text("UUID"),
    ]
}

/// Join a `string[]` property — vim25 names those elements `<string>`.
fn string_array(host: &ManagedObject, prop: &str) -> Option<String> {
    let joined = host
        .array_prop(prop)
        .iter()
        .map(|e| e.text.as_str())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    (!joined.is_empty()).then_some(joined)
}

/// Whether a host service is running, by its `HostService` key.
fn service_running(host: &ManagedObject, key: &str) -> Option<bool> {
    host.array_prop("config.service.service")
        .iter()
        .find(|s| s.text_at("key").as_deref() == Some(key))
        .and_then(|s| match s.text_at("running").as_deref() {
            Some("true") => Some(true),
            Some("false") => Some(false),
            _ => None,
        })
}

/// One entry of `hardware.systemInfo.otherIdentifyingInfo`, by identifier key
/// (`ServiceTag`, `AssetTag`, …).
fn identifying_info(host: &ManagedObject, key: &str) -> Option<String> {
    host.array_prop("hardware.systemInfo.otherIdentifyingInfo")
        .iter()
        .find(|e| e.text_at("identifierType/key").as_deref() == Some(key))
        .and_then(|e| e.text_at("identifierValue"))
        .filter(|v| !v.is_empty() && v != "Default string")
}

/// Size of one memory tier, by `HostMemoryTierInfo` type (`DRAM`, `NVMe`).
///
/// With memory tiering enabled — the default on this hardware in vSphere 9 —
/// `hardware.memorySize` is DRAM *plus* the NVMe tier, so a host reporting
/// 478 GiB may hold only 96 GiB of DRAM. The tiers are broken out rather than
/// letting one number imply physical RAM.
fn memory_tier_bytes(host: &ManagedObject, tier_type: &str) -> Option<i64> {
    host.array_prop("hardware.memoryTierInfo")
        .iter()
        .find(|t| t.text_at("type").as_deref() == Some(tier_type))
        .and_then(|t| t.text_at("size"))
        .and_then(|s| s.parse().ok())
}

/// The tier types present, e.g. `DRAM` or `DRAM + NVMe`.
fn memory_tiering_type(host: &ManagedObject) -> Option<String> {
    let types: Vec<String> = host
        .array_prop("hardware.memoryTierInfo")
        .iter()
        .filter_map(|t| t.text_at("type"))
        .filter(|t| !t.is_empty())
        .collect();
    (!types.is_empty()).then(|| types.join(" + "))
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    // The per-host VM rollup used to be its own inventory walk. It now reads
    // the same VM list every other sheet in the fetch shares.
    let vm_totals = vm_totals_by_host(&snap.vms);

    let mut rows = Vec::with_capacity(snap.hosts.len());
    for host in &snap.hosts {
        let Some(name) = host.str_prop("name") else {
            return Err(format!("HostSystem {} returned no name property", host.moref));
        };

        let cores = host.i64_prop("summary.hardware.numCpuCores");
        let sockets = host.i64_prop("summary.hardware.numCpuPkgs");
        let memory_bytes = host.i64_prop("summary.hardware.memorySize");
        let totals = vm_totals.get(&host.moref).cloned().unwrap_or(HostVmTotals::default());

        // Host memory is reported in bytes and quickStats in MiB, so the usage
        // percentage has to convert before dividing.
        let memory_usage_pct = match (host.i64_prop("summary.quickStats.overallMemoryUsage"), memory_bytes) {
            (Some(used_mib), Some(total_bytes)) if total_bytes > 0 => {
                let total_mib = total_bytes / (1024 * 1024);
                Some((used_mib as f64 / total_mib as f64 * 10000.0).round() / 100.0)
            }
            _ => None,
        };
        // Total host CPU capacity is cores × per-core MHz.
        let cpu_capacity_mhz = match (cores, host.i64_prop("summary.hardware.cpuMhz")) {
            (Some(c), Some(mhz)) => Some(c * mhz),
            _ => None,
        };
        let cpu_usage_pct = match (host.i64_prop("summary.quickStats.overallCpuUsage"), cpu_capacity_mhz) {
            (Some(used), Some(total)) if total > 0 => {
                Some((used as f64 / total as f64 * 10000.0).round() / 100.0)
            }
            _ => None,
        };

        rows.push((host.moref.clone(), vec![
            Cell::Text(name),
            Cell::opt_text(host.str_prop("overallStatus")),
            Cell::opt_bool(host.bool_prop("runtime.inMaintenanceMode")),
            Cell::opt_text(host.str_prop("runtime.connectionState")),
            Cell::opt_text(host.str_prop("runtime.powerState")),
            Cell::opt_text(host.str_prop("summary.hardware.cpuModel")),
            Cell::opt_num(host.i64_prop("summary.hardware.cpuMhz").map(|v| v as f64)),
            Cell::opt_bool(host.bool_prop("config.hyperThread.available")),
            Cell::opt_bool(host.bool_prop("config.hyperThread.active")),
            Cell::opt_num(sockets.map(|v| v as f64)),
            Cell::opt_num(match (cores, sockets) {
                (Some(c), Some(s)) if s > 0 => Some((c / s) as f64),
                _ => None,
            }),
            Cell::opt_num(cores.map(|v| v as f64)),
            Cell::opt_num(host.i64_prop("summary.hardware.numCpuThreads").map(|v| v as f64)),
            Cell::opt_num(cpu_usage_pct),
            Cell::opt_num(memory_bytes.map(bytes_to_gib)),
            Cell::opt_text(memory_tiering_type(host)),
            Cell::opt_num(memory_tier_bytes(host, "DRAM").map(bytes_to_gib)),
            Cell::opt_num(memory_tier_bytes(host, "NVMe").map(bytes_to_gib)),
            Cell::opt_num(memory_usage_pct),
            Cell::opt_num(host.i64_prop("summary.hardware.numNics").map(|v| v as f64)),
            Cell::opt_num(host.i64_prop("summary.hardware.numHBAs").map(|v| v as f64)),
            Cell::Number(totals.vms_total as f64),
            Cell::Number(totals.vms_powered_on as f64),
            Cell::opt_num(ratio(totals.vms_total, cores)),
            Cell::Number(totals.vcpus as f64),
            Cell::opt_num(ratio(totals.vcpus, cores)),
            Cell::Number(bytes_to_gib(totals.vram_mib * 1024 * 1024)),
            Cell::opt_bool(host.bool_prop("capability.vmotionSupported")),
            Cell::opt_bool(host.bool_prop("capability.storageVMotionSupported")),
            // Absent on hosts not in an EVC-enabled cluster — left empty rather
            // than filled with a placeholder.
            Cell::opt_text(host.str_prop("summary.currentEVCModeKey")),
            Cell::opt_text(host.str_prop("summary.maxEVCModeKey")),
            Cell::opt_text(host.str_prop("hardware.cpuPowerManagementInfo.currentPolicy")),
            Cell::opt_text(host.str_prop("config.product.fullName")),
            Cell::opt_text(host.str_prop("runtime.bootTime")),
            Cell::opt_text(string_array(host, "config.network.dnsConfig.address")),
            Cell::opt_bool(host.bool_prop("config.network.dnsConfig.dhcp")),
            Cell::opt_text(host.str_prop("config.network.dnsConfig.domainName")),
            Cell::opt_text(string_array(host, "config.network.dnsConfig.searchDomain")),
            Cell::opt_text(string_array(host, "config.dateTimeInfo.ntpConfig.server")),
            Cell::opt_bool(service_running(host, "ntpd")),
            Cell::opt_text(host.str_prop("config.dateTimeInfo.timeZone.name")),
            Cell::opt_num(host.i64_prop("config.dateTimeInfo.timeZone.gmtOffset").map(|v| v as f64)),
            Cell::opt_text(host.str_prop("summary.hardware.vendor")),
            Cell::opt_text(host.str_prop("summary.hardware.model")),
            Cell::opt_text(host.str_prop("hardware.systemInfo.serialNumber")),
            Cell::opt_text(identifying_info(host, "ServiceTag")),
            Cell::opt_text(host.str_prop("hardware.biosInfo.biosVersion")),
            Cell::opt_text(host.str_prop("hardware.biosInfo.releaseDate")),
            Cell::opt_text(host.str_prop("summary.hardware.uuid")),
        ]));
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vHost",
    columns,
    // vHost is host-derived, but its `# VMs` / `# vCPUs` / `vRAM` columns are a
    // rollup over VMs, so it declares both sets.
    vm_props: &[VM_TOTALS_PROPS],
    host_props: &[HOST_PROPS],
    dvs_props: &[],
    dvpg_props: &[],
    source: RowSource::Host,
    rows,
};

pub async fn fetch_vhost_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{cells, col, host, vm};

    fn cell(rows: &[Vec<Cell>], row: usize, label: &str) -> Cell {
        rows[row][col(&columns(), label)].clone()
    }

    /// vHost's VM columns used to cost their own inventory walk. They now read
    /// the same VM list every other sheet in the fetch shares, so this asserts
    /// the rollup still lands on the right host.
    #[test]
    fn vm_columns_roll_up_from_the_shared_snapshot() {
        let snap = InventorySnapshot::from_parts(
            vec![
                vm(
                    "vm-1",
                    &[
                        ("name", "OPS91"),
                        ("runtime.host", "host-1"),
                        ("runtime.powerState", "poweredOn"),
                        ("config.hardware.numCPU", "4"),
                        ("config.hardware.memoryMB", "8192"),
                    ],
                ),
                vm(
                    "vm-2",
                    &[
                        ("name", "OPS92"),
                        ("runtime.host", "host-1"),
                        ("runtime.powerState", "poweredOff"),
                        ("config.hardware.numCPU", "2"),
                        ("config.hardware.memoryMB", "4096"),
                    ],
                ),
                // Registered elsewhere: must not count against host-1.
                vm(
                    "vm-3",
                    &[
                        ("name", "OTHER"),
                        ("runtime.host", "host-2"),
                        ("runtime.powerState", "poweredOn"),
                        ("config.hardware.numCPU", "8"),
                        ("config.hardware.memoryMB", "16384"),
                    ],
                ),
            ],
            vec![host(
                "host-1",
                &[("name", "esx9-01.example.com"), ("summary.hardware.numCpuCores", "6")],
            )],
        );

        let rows = cells(rows(&snap).expect("rows build"));
        assert_eq!(rows.len(), 1);
        assert!(matches!(cell(&rows, 0, "# VMs total"), Cell::Number(v) if v == 2.0));
        assert!(matches!(cell(&rows, 0, "# VMs"), Cell::Number(v) if v == 1.0));
        assert!(matches!(cell(&rows, 0, "# vCPUs"), Cell::Number(v) if v == 6.0));
        // 8192 + 4096 MiB = 12 GiB.
        assert!(matches!(cell(&rows, 0, "vRAM GiB"), Cell::Number(v) if v == 12.0));
        // 6 vCPUs over 6 physical cores.
        assert!(matches!(cell(&rows, 0, "vCPUs per Core"), Cell::Number(v) if v == 1.0));
    }

    /// vCLS VMs are vSphere-managed; vSphere's own count excludes them, so the
    /// rollup must too.
    #[test]
    fn vcls_vms_do_not_count_against_a_host() {
        let snap = InventorySnapshot::from_parts(
            vec![vm(
                "vm-1",
                &[
                    ("name", "vCLS-4f2e"),
                    ("runtime.host", "host-1"),
                    ("runtime.powerState", "poweredOn"),
                    ("config.hardware.numCPU", "1"),
                ],
            )],
            vec![host("host-1", &[("name", "esx9-01.example.com")])],
        );

        let rows = cells(rows(&snap).expect("rows build"));
        assert!(matches!(cell(&rows, 0, "# VMs total"), Cell::Number(v) if v == 0.0));
    }

    #[test]
    fn a_host_without_a_name_is_an_error() {
        let snap = InventorySnapshot::from_parts(Vec::new(), vec![host("host-9", &[])]);
        let err = rows(&snap).expect_err("a nameless host is reported");
        assert!(err.contains("host-9"), "the error names the object: {err}");
    }
}

/// vHost over a real captured `HostSystem` response.
#[cfg(test)]
mod captured_tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_snapshot, cells, col};

    fn at(rows: &[Vec<Cell>], label: &str) -> Cell {
        rows[0][col(&columns(), label)].clone()
    }

    #[test]
    fn one_row_per_captured_host() {
        let rows = cells(rows(&captured_snapshot()).expect("captured host has a name"));
        assert_eq!(rows.len(), 1);
        assert!(matches!(at(&rows, "Host"), Cell::Text(ref s) if s == "esx01.lab.local"));
    }

    #[test]
    fn hardware_columns_match_the_capture() {
        let rows = cells(rows(&captured_snapshot()).expect("named host"));
        assert!(matches!(at(&rows, "Vendor"), Cell::Text(ref s) if s == "HPE"));
        assert!(matches!(at(&rows, "Model"), Cell::Text(ref s) if s == "ProLiant DL380 Gen10"));
        assert!(matches!(at(&rows, "# Cores"), Cell::Number(n) if n == 48.0));
        assert!(
            matches!(at(&rows, "ESX Version"), Cell::Text(ref s) if s.starts_with("VMware ESXi 9.1.0"))
        );
        // 824269418496 bytes, converted to GiB and rounded to two places.
        assert!(matches!(at(&rows, "# Memory GiB"), Cell::Number(n) if (n - 767.66).abs() < 0.01));
    }

    /// `config.service.service` is an array of `HostService`; ntpd is found by
    /// its `key`, never by position.
    #[test]
    fn ntpd_state_is_read_from_the_service_array() {
        let rows = cells(rows(&captured_snapshot()).expect("named host"));
        assert!(matches!(at(&rows, "NTPD running"), Cell::Bool(true)));
        assert!(matches!(at(&rows, "NTP Server(s)"), Cell::Text(ref s) if !s.is_empty()));
    }

    /// Memory tiering reports DRAM only in this lab, so the NVMe tier column is
    /// empty by fact rather than by parse failure. See docs/LAB-ENVIRONMENT.md.
    #[test]
    fn a_dram_only_host_reports_no_nvme_tier() {
        let rows = cells(rows(&captured_snapshot()).expect("named host"));
        assert!(matches!(at(&rows, "NVMe Tier GiB"), Cell::Empty));
    }

    /// Only the VM whose `runtime.host` matches the captured host counts toward
    /// its rollup; the other three captures point at an absent host.
    #[test]
    fn vm_rollup_counts_only_vms_on_that_host() {
        let rows = cells(rows(&captured_snapshot()).expect("named host"));
        // Two of the five captured VMs run on the captured host; the rest name
        // a host that is deliberately absent from the corpus.
        assert!(
            matches!(at(&rows, "# VMs total"), Cell::Number(n) if n == 2.0),
            "got {:?}",
            at(&rows, "# VMs total")
        );
    }
}
