//! vHealth — one row per detected health issue.
//!
//! **This is not alarm or event aggregation.** RVTools computes vHealth itself
//! from the inventory it has already collected: the reference export's rows are
//! all NTP, NTPD, Foldername, CDROM, Snapshot, Zombie and "Performance tip"
//! findings, with no vCenter alarm anywhere in the sheet. Messages below
//! reproduce RVTools' exact wording, read out of that export.
//!
//! Two of RVTools' checks are deliberately not implemented:
//!
//! - **Zombie** (`Possibly a Zombie vmdk file! Please check.`) needs
//!   `HostDatastoreBrowser` / `SearchDatastoreSubFolders` to walk datastore
//!   file trees and diff them against registered disks — a different API area
//!   entirely, and out of scope per CLAUDE.md.
//! - **Performance tip** (`In-Memory VM performance improvement possible!`)
//!   appears twice in the reference export with no property that reliably
//!   distinguishes those VMs from the rest. Guessing a trigger would produce
//!   findings that are confidently wrong, which is worse than a missing check.

use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::soap::ManagedObject;
use crate::vcenter::xml::Element;
use crate::vcenter::VCenterConnection;

/// `HostSystem` properties this sheet reads.
pub const HOST_PROPS: &[&str] = &[
    "name",
    "config.dateTimeInfo.ntpConfig.server",
    "config.service.service",
];

/// `VirtualMachine` properties this sheet reads.
pub const VM_PROPS: &[&str] = &[
    "name",
    "config.files.vmPathName",
    "config.hardware.device",
    "snapshot.rootSnapshotList",
];

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("Name"),
        Column::text("Message"),
        Column::text("Message type"),
    ]
}

/// vHealth shows no location columns, but rows still carry their source moref
/// so the sheet contract is uniform across every sheet.
fn row(moref: &str, name: &str, message: String, kind: &str) -> (String, Vec<Cell>) {
    (
        moref.to_string(),
        vec![
            Cell::Text(name.to_string()),
            Cell::Text(message),
            Cell::Text(kind.to_string()),
        ],
    )
}

/// The folder component of `[datastore] folder/name.vmx`.
///
/// Returns `None` for a VM stored at the datastore root, which has no folder to
/// disagree with.
fn folder_of(vm_path: &str) -> Option<&str> {
    let after_datastore = vm_path.split_once("] ").map(|(_, rest)| rest)?;
    let (folder, _file) = after_datastore.rsplit_once('/')?;
    Some(folder)
}

/// RVTools renders snapshot timestamps as `yyyy/MM/dd HH:mm:ss`.
fn format_created(raw: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.naive_utc().format("%Y/%m/%d %H:%M:%S").to_string())
        .unwrap_or_else(|_| raw.to_string())
}

/// Every snapshot in the tree, flattened; nested snapshots hang off
/// `childSnapshotList`.
fn walk_snapshots(node: &Element, out: &mut Vec<(String, String)>) {
    if let Some(name) = node.text_at("name") {
        out.push((name, node.text_at("createTime").unwrap_or_default()));
    }
    for child in node.children_named("childSnapshotList") {
        walk_snapshots(child, out);
    }
}

fn host_findings(
    host: &ManagedObject,
    rows: &mut Vec<(String, Vec<Cell>)>,
) -> Result<(), String> {
    let Some(name) = host.str_prop("name") else {
        return Err(format!("HostSystem {} returned no name property", host.moref));
    };

    let ntp_servers: Vec<&Element> = host.array_prop("config.dateTimeInfo.ntpConfig.server");
    if ntp_servers.iter().all(|s| s.text.is_empty()) {
        rows.push(row(&host.moref, &name, "NTP Server value is null!".into(), "NTP"));
    }

    let ntpd_running = host
        .array_prop("config.service.service")
        .iter()
        .find(|s| s.text_at("key").as_deref() == Some("ntpd"))
        .and_then(|s| s.text_at("running"))
        .map(|v| v == "true")
        // A host that does not report the service at all is not running it.
        .unwrap_or(false);
    if !ntpd_running {
        rows.push(row(&host.moref, &name, "NTPD service is not running!".into(), "NTPD"));
    }

    Ok(())
}

fn vm_findings(
    vm: &ManagedObject,
    rows: &mut Vec<(String, Vec<Cell>)>,
) -> Result<(), String> {
    let Some(name) = vm.str_prop("name") else {
        return Err(format!("VirtualMachine {} returned no name property", vm.moref));
    };
    if name.starts_with("vCLS-") {
        return Ok(());
    }

    // Foldername: the comparison is case-sensitive — RVTools flags "FLASK" in
    // folder "flask".
    if let Some(folder) = vm.str_prop("config.files.vmPathName").as_deref().and_then(folder_of) {
        if folder != name {
            rows.push(row(
                &vm.moref,
                &name,
                format!("Inconsistent Foldername! VMname = {name} Foldername = {folder}"),
                "Foldername",
            ));
        }
    }

    // CDROM: only CD-ROM devices, and only those actually connected. Ethernet
    // cards carry a `connectable` block too, so filtering on that alone would
    // report every powered-on NIC as a mounted CD.
    for cdrom in vm
        .array_prop("config.hardware.device")
        .iter()
        .filter(|d| d.xsi_type.as_deref() == Some("VirtualCdrom"))
    {
        if cdrom.text_at("connectable/connected").as_deref() == Some("true") {
            let label = cdrom.text_at("deviceInfo/label").unwrap_or_default();
            rows.push(row(
                &vm.moref,
                &name,
                format!("VM has a CDROM device connected! {label}"),
                "CDROM",
            ));
        }
    }

    let mut snapshots = Vec::new();
    for root in vm.array_prop("snapshot.rootSnapshotList") {
        walk_snapshots(root, &mut snapshots);
    }
    for (snap_name, created) in snapshots {
        rows.push(row(
            &vm.moref,
            &name,
            format!(
                "VM has an active snapshot! {snap_name} created on {}",
                format_created(&created)
            ),
            "Snapshot",
        ));
    }

    Ok(())
}

/// Hosts first, then VMs — the order RVTools emits its own checks in.
pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let mut rows = Vec::new();

    for host in &snap.hosts {
        host_findings(host, &mut rows)?;
    }
    for vm in &snap.vms {
        vm_findings(vm, &mut rows)?;
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vHealth",
    columns,
    vm_props: &[VM_PROPS],
    host_props: &[HOST_PROPS],
    // RVTools' vHealth is three columns wide: Name, Message, Message type.
    // It gets no location columns.
    source: RowSource::None,
    rows,
};

pub async fn fetch_vhealth_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcenter::xml;

    /// Build a `ManagedObject` from an `<objects>` fragment, the way a real
    /// `RetrievePropertiesEx` response is shaped.
    fn object(props: &str) -> ManagedObject {
        let xml = format!(
            r#"<objects><obj type="X">x-1</obj>{props}</objects>"#
        );
        ManagedObject::from_element(&xml::parse(&xml).expect("fragment parses"))
    }

    fn prop(name: &str, val: &str) -> String {
        format!("<propSet><name>{name}</name><val>{val}</val></propSet>")
    }

    fn messages(rows: &[(String, Vec<Cell>)]) -> Vec<(String, String)> {
        rows.iter()
            .map(|(_, r)| match (&r[1], &r[2]) {
                (Cell::Text(m), Cell::Text(t)) => (t.clone(), m.clone()),
                _ => panic!("message and type are always text"),
            })
            .collect()
    }

    #[test]
    fn folder_is_taken_from_the_vmx_path() {
        assert_eq!(folder_of("[SYN-HDD] OPS91/OPS91.vmx"), Some("OPS91"));
        assert_eq!(folder_of("[SYN-HDD] a/b/c.vmx"), Some("a/b"));
        // A VM at the datastore root has no folder to disagree with.
        assert_eq!(folder_of("[SYN-HDD] OPS91.vmx"), None);
        assert_eq!(folder_of("nonsense"), None);
    }

    #[test]
    fn snapshot_timestamps_use_rvtools_format() {
        assert_eq!(
            format_created("2026-08-27T16:17:12.873279Z"),
            "2026/08/27 16:17:12"
        );
        // Unparseable input is passed through rather than dropped.
        assert_eq!(format_created("not a date"), "not a date");
    }

    /// The lab's hosts all have NTP configured, so this path never runs against
    /// live data.
    #[test]
    fn host_without_ntp_servers_is_reported() {
        let host = object(&format!(
            "{}{}",
            prop("name", "esx1.example.com"),
            prop("config.dateTimeInfo.ntpConfig.server", "")
        ));
        let mut rows = Vec::new();
        host_findings(&host, &mut rows).expect("named host");
        assert_eq!(
            messages(&rows),
            vec![
                ("NTP".into(), "NTP Server value is null!".to_string()),
                ("NTPD".into(), "NTPD service is not running!".to_string()),
            ]
        );
    }

    #[test]
    fn host_with_ntp_and_running_ntpd_is_clean() {
        let host = object(&format!(
            "{}{}{}",
            prop("name", "esx1.example.com"),
            prop(
                "config.dateTimeInfo.ntpConfig.server",
                "<string>time.example.com</string>"
            ),
            prop(
                "config.service.service",
                "<HostService><key>ntpd</key><running>true</running></HostService>"
            )
        ));
        let mut rows = Vec::new();
        host_findings(&host, &mut rows).expect("named host");
        assert!(rows.is_empty(), "clean host produced {rows:?}");
    }

    #[test]
    fn only_connected_cdroms_are_reported() {
        let vm = object(&format!(
            "{}{}",
            prop("name", "VM1"),
            prop(
                "config.hardware.device",
                concat!(
                    r#"<VirtualDevice xsi:type="VirtualCdrom"><deviceInfo><label>CD/DVD drive 1</label></deviceInfo><connectable><connected>true</connected></connectable></VirtualDevice>"#,
                    r#"<VirtualDevice xsi:type="VirtualCdrom"><deviceInfo><label>CD/DVD drive 2</label></deviceInfo><connectable><connected>false</connected></connectable></VirtualDevice>"#,
                    // An ethernet card also has a connectable block; it must not
                    // be mistaken for a mounted CD.
                    r#"<VirtualDevice xsi:type="VirtualE1000e"><deviceInfo><label>Network adapter 1</label></deviceInfo><connectable><connected>true</connected></connectable></VirtualDevice>"#,
                )
            )
        ));
        let mut rows = Vec::new();
        vm_findings(&vm, &mut rows).expect("named vm");
        assert_eq!(
            messages(&rows),
            vec![(
                "CDROM".into(),
                "VM has a CDROM device connected! CD/DVD drive 1".to_string()
            )]
        );
    }

    /// The lab has no nested snapshots, so the recursion is only exercised here.
    #[test]
    fn nested_snapshots_are_flattened() {
        let vm = object(&format!(
            "{}{}",
            prop("name", "VM1"),
            prop(
                "snapshot.rootSnapshotList",
                concat!(
                    "<VirtualMachineSnapshotTree><name>root</name>",
                    "<createTime>2026-01-01T00:00:00Z</createTime>",
                    "<childSnapshotList><name>child</name>",
                    "<createTime>2026-01-02T03:04:05Z</createTime>",
                    "<childSnapshotList><name>grandchild</name>",
                    "<createTime>2026-01-03T00:00:00Z</createTime></childSnapshotList>",
                    "</childSnapshotList></VirtualMachineSnapshotTree>",
                )
            )
        ));
        let mut rows = Vec::new();
        vm_findings(&vm, &mut rows).expect("named vm");
        let msgs = messages(&rows);
        assert_eq!(msgs.len(), 3, "root, child and grandchild: {msgs:?}");
        assert_eq!(
            msgs[1].1,
            "VM has an active snapshot! child created on 2026/01/02 03:04:05"
        );
        assert!(msgs[2].1.contains("grandchild"));
    }

    #[test]
    fn folder_mismatch_is_case_sensitive() {
        let vm = object(&format!(
            "{}{}",
            prop("name", "FLASK"),
            prop("config.files.vmPathName", "[DS1] flask/FLASK.vmx")
        ));
        let mut rows = Vec::new();
        vm_findings(&vm, &mut rows).expect("named vm");
        assert_eq!(
            messages(&rows),
            vec![(
                "Foldername".into(),
                "Inconsistent Foldername! VMname = FLASK Foldername = flask".to_string()
            )]
        );
    }

    #[test]
    fn vcls_vms_are_skipped() {
        let vm = object(&format!(
            "{}{}",
            prop("name", "vCLS-abc"),
            prop("config.files.vmPathName", "[DS1] other/vCLS-abc.vmx")
        ));
        let mut rows = Vec::new();
        vm_findings(&vm, &mut rows).expect("named vm");
        assert!(rows.is_empty());
    }
}

/// vHealth over real captured responses.
#[cfg(test)]
mod captured_tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_snapshot, cells};

    fn findings(rows: &[Vec<Cell>]) -> Vec<(String, String)> {
        rows.iter()
            .map(|r| match (&r[2], &r[0]) {
                (Cell::Text(kind), Cell::Text(name)) => (kind.clone(), name.clone()),
                _ => panic!("name and type are always text"),
            })
            .collect()
    }

    /// The captured host is healthy (NTP configured, ntpd running), so every
    /// finding comes from a VM.
    #[test]
    fn a_healthy_captured_host_contributes_no_findings() {
        let rows = cells(rows(&captured_snapshot()).expect("named objects"));
        let f = findings(&rows);
        assert!(
            !f.iter().any(|(kind, _)| kind == "NTP" || kind == "NTPD"),
            "captured host has NTP set and ntpd running, got {f:?}"
        );
    }

    /// On vSAN every VM folder is an object UUID, so it can never equal the VM
    /// name and the check fires for all four captures. That is RVTools' rule
    /// reproduced faithfully, not a defect. See docs/LAB-ENVIRONMENT.md.
    #[test]
    fn vsan_uuid_folders_trip_foldername_for_every_vm() {
        let rows = cells(rows(&captured_snapshot()).expect("named objects"));
        let folder: Vec<_> = findings(&rows)
            .into_iter()
            .filter(|(kind, _)| kind == "Foldername")
            .collect();
        assert_eq!(folder.len(), 5, "got {folder:?}");
    }

    /// One capture has a connected CD-ROM. The others carry a disconnected
    /// `VirtualCdrom`, and every VM carries NICs that have their own
    /// `connectable` block; filtering on `connectable` alone would report those
    /// too.
    #[test]
    fn only_the_connected_cdrom_is_reported() {
        let rows = cells(rows(&captured_snapshot()).expect("named objects"));
        let cd: Vec<_> = findings(&rows).into_iter().filter(|(k, _)| k == "CDROM").collect();
        assert_eq!(cd.len(), 1, "got {cd:?}");
        assert_eq!(cd[0].1, "k8s-controller-01");
    }

    #[test]
    fn every_snapshot_is_reported_including_nested_ones() {
        let rows = cells(rows(&captured_snapshot()).expect("named objects"));
        let snaps: Vec<_> =
            findings(&rows).into_iter().filter(|(k, _)| k == "Snapshot").collect();
        assert_eq!(snaps.len(), 3, "got {snaps:?}");
        assert!(snaps.iter().any(|(_, n)| n == "vSAN File Service Node (1)"));
        assert_eq!(
            snaps.iter().filter(|(_, n)| n == "sttools-fixture-01").count(),
            2,
            "the nested child must be reported as well as its parent"
        );
    }

    #[test]
    fn total_findings_for_the_capture() {
        let rows = cells(rows(&captured_snapshot()).expect("named objects"));
        // 5 Foldername + 1 CDROM + 3 Snapshot, and nothing from the host.
        assert_eq!(rows.len(), 9);
    }
}

/// vHealth's snapshot check over a real nested snapshot.
#[cfg(test)]
mod nested_capture_tests {
    use super::*;
    use crate::data::snapshot::test_support::captured_snapshot;

    /// Both the parent and its nested child are reported. A flattener that
    /// stopped at the roots would find only one and no error would be raised.
    #[test]
    fn a_nested_snapshot_is_reported_as_well_as_its_parent() {
        let rows = rows(&captured_snapshot()).expect("named objects");
        let found: Vec<String> = rows
            .iter()
            .filter(|(_, r)| matches!(&r[2], Cell::Text(k) if k == "Snapshot"))
            .filter_map(|(_, r)| match (&r[0], &r[1]) {
                (Cell::Text(name), Cell::Text(msg)) if name == "sttools-fixture-01" => {
                    Some(msg.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(found.len(), 2, "parent and child, got {found:?}");
        assert!(found.iter().any(|m| m.contains("sttools-parent")));
        assert!(found.iter().any(|m| m.contains("sttools-child")));
    }
}
