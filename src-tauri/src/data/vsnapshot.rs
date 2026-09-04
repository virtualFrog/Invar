//! vSnapshot — one row per snapshot.
//!
//! Sourced from `snapshot.rootSnapshotList`, whose elements are
//! `<VirtualMachineSnapshotTree>`. Sizes come from `layoutEx`: each snapshot's
//! `dataKey` and `memoryKey` index into `layoutEx.file`.

use super::common::{bytes_to_mib, VmContext, VM_CONTEXT_PROPS};
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::soap::ManagedObject;
use crate::vcenter::xml::Element;
use crate::vcenter::VCenterConnection;
use std::collections::HashMap;

/// What this sheet reads beyond `common::VM_CONTEXT_PROPS`: the snapshot tree,
/// and the `layoutEx` files the per-snapshot sizes index into.
pub const VM_PROPS: &[&str] = &[
    "snapshot.rootSnapshotList",
    "snapshot.currentSnapshot",
    "layoutEx.file",
    "layoutEx.snapshot",
    "config.uuid",
];

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("VM"),
        Column::text("Powerstate"),
        Column::text("Name"),
        Column::text("Description"),
        Column::text("Date / time"),
        Column::text("Filename"),
        Column::number("Size MiB (vmsn)"),
        Column::number("Size MiB (total)"),
        Column::bool("Quiesced"),
        Column::text("State"),
        Column::bool("Is current"),
        Column::text("Host"),
        Column::text("Annotation"),
        // ---- Phase 4 ----
        Column::text("VM ID"),
        Column::text("VM UUID"),
    ]
}

/// One snapshot, flattened out of the tree.
struct Snapshot {
    moref: String,
    name: Option<String>,
    description: Option<String>,
    create_time: Option<String>,
    state: Option<String>,
    quiesced: Option<bool>,
}

/// Flatten `rootSnapshotList` depth-first.
///
/// Snapshots nest: a child is a `<childSnapshotList>` element inside its parent
/// tree node. (Unlike a top-level property array, whose members are named after
/// the declared type, nested arrays inside a data object repeat the *field*
/// name — as `<chain>`/`<fileKey>` do in `layoutEx.snapshot`.)
fn flatten(node: &Element, out: &mut Vec<Snapshot>) {
    out.push(Snapshot {
        moref: node.text_at("snapshot").unwrap_or_default(),
        name: node.text_at("name").filter(|s| !s.is_empty()),
        description: node.text_at("description").filter(|s| !s.is_empty()),
        create_time: node.text_at("createTime").filter(|s| !s.is_empty()),
        state: node.text_at("state").filter(|s| !s.is_empty()),
        quiesced: match node.text_at("quiesced").as_deref() {
            Some("true") => Some(true),
            Some("false") => Some(false),
            _ => None,
        },
    });
    for child in node.children_named("childSnapshotList") {
        flatten(child, out);
    }
}

/// `layoutEx.file` key → (name, size in bytes).
fn file_layout(vm: &ManagedObject) -> HashMap<String, (String, i64)> {
    vm.array_prop("layoutEx.file")
        .iter()
        .filter_map(|f| {
            let key = f.text_at("key")?;
            let name = f.text_at("name")?;
            let size = f.text_at("size")?.parse().ok()?;
            Some((key, (name, size)))
        })
        .collect()
}

/// Snapshot moref → its `dataKey` and `memoryKey` file references.
fn snapshot_layout(vm: &ManagedObject) -> HashMap<String, (Option<String>, Option<String>)> {
    vm.array_prop("layoutEx.snapshot")
        .iter()
        .filter_map(|s| {
            let key = s.text_at("key")?;
            Some((key, (s.text_at("dataKey"), s.text_at("memoryKey"))))
        })
        .collect()
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let hosts = &snap.host_names;

    let mut rows = Vec::new();
    for vm in &snap.vms {
        let Some(ctx) = VmContext::from(vm, hosts)? else {
            continue;
        };

        let mut snapshots = Vec::new();
        for root in vm.array_prop("snapshot.rootSnapshotList") {
            flatten(root, &mut snapshots);
        }
        if snapshots.is_empty() {
            continue;
        }

        let files = file_layout(vm);
        let layout = snapshot_layout(vm);
        let current = vm.str_prop("snapshot.currentSnapshot");

        for snap in snapshots {
            // A memoryKey of -1 means the snapshot captured no memory state.
            let (data_key, memory_key) = layout.get(&snap.moref).cloned().unwrap_or((None, None));
            let file_of = |key: &Option<String>| {
                key.as_ref()
                    .filter(|k| *k != "-1")
                    .and_then(|k| files.get(k))
                    .cloned()
            };
            let data_file = file_of(&data_key);
            let memory_file = file_of(&memory_key);

            // "Size MiB (total)" is the snapshot's own files: the .vmsn plus a
            // .vmem when memory was captured. Delta (redo-log) growth is not
            // included — it is not attributable to a single snapshot from
            // layoutEx, and guessing would overstate every row.
            let total_bytes = match (&data_file, &memory_file) {
                (None, None) => None,
                (a, b) => Some(a.as_ref().map_or(0, |f| f.1) + b.as_ref().map_or(0, |f| f.1)),
            };

            rows.push((vm.moref.clone(), vec![
                Cell::Text(ctx.name.clone()),
                Cell::opt_text(ctx.power_state.clone()),
                Cell::opt_text(snap.name),
                Cell::opt_text(snap.description),
                Cell::opt_text(snap.create_time),
                Cell::opt_text(data_file.as_ref().map(|f| f.0.clone())),
                Cell::opt_num(data_file.as_ref().map(|f| bytes_to_mib(f.1))),
                Cell::opt_num(total_bytes.map(bytes_to_mib)),
                Cell::opt_bool(snap.quiesced),
                Cell::opt_text(snap.state),
                Cell::Bool(current.as_deref() == Some(snap.moref.as_str())),
                Cell::opt_text(ctx.host.clone()),
                Cell::opt_text(ctx.annotation.clone()),
                // ---- Phase 4 ----
                Cell::Text(vm.moref.clone()),
                Cell::opt_text(vm.str_prop("config.uuid")),
            ]));
        }
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vSnapshot",
    columns,
    vm_props: &[VM_CONTEXT_PROPS, VM_PROPS],
    host_props: &[],
    dvs_props: &[],
    dvpg_props: &[],
    cluster_props: &[],
    datastore_props: &[],
    rp_props: &[],
    wants_licenses: false,
    wants_about: false,
    wants_files: false,
    source: RowSource::Vm,
    rows,
};

pub async fn fetch_vsnapshot_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcenter::xml;

    /// The lab has no nested snapshots, so the recursion through
    /// `childSnapshotList` cannot be exercised against live data.
    #[test]
    fn nested_snapshots_are_flattened_depth_first() {
        let tree = xml::parse(concat!(
            "<VirtualMachineSnapshotTree><snapshot>snapshot-1</snapshot><name>root</name>",
            "<createTime>2026-01-01T00:00:00Z</createTime><state>poweredOn</state>",
            "<childSnapshotList><snapshot>snapshot-2</snapshot><name>child</name>",
            "<createTime>2026-01-02T00:00:00Z</createTime><quiesced>true</quiesced>",
            "</childSnapshotList>",
            "<childSnapshotList><snapshot>snapshot-3</snapshot><name>sibling</name>",
            "</childSnapshotList></VirtualMachineSnapshotTree>",
        ))
        .expect("fragment parses");

        let mut out = Vec::new();
        flatten(&tree, &mut out);

        let names: Vec<_> = out.iter().map(|s| s.name.clone().unwrap_or_default()).collect();
        assert_eq!(names, vec!["root", "child", "sibling"]);
        assert_eq!(out[0].moref, "snapshot-1");
        assert_eq!(out[1].quiesced, Some(true));
        assert_eq!(out[2].create_time, None);
    }
}

/// vSnapshot over real captured responses.
#[cfg(test)]
mod captured_tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_snapshot, cells, col};

    /// One of the four captured VMs carries a snapshot, so the sheet is one
    /// row: vSnapshot is per-snapshot, not per-VM.
    #[test]
    fn only_vms_with_snapshots_produce_rows() {
        let rows = cells(rows(&captured_snapshot()).expect("named VMs"));
        // Two VMs carry snapshots, and one of those has a nested pair.
        assert_eq!(rows.len(), 3);
        let vm_col = col(&columns(), "VM");
        assert!(rows.iter().any(
            |r| matches!(&r[vm_col], Cell::Text(n) if n == "vSAN File Service Node (1)")
        ));
    }

    /// `snapshot.rootSnapshotList` is a `VirtualMachineSnapshotTree[]`, so its
    /// elements carry the TYPE name. Reading the field name instead yields zero
    /// rows and no error, which is the failure this capture exists to catch.
    #[test]
    fn the_snapshot_fields_come_off_the_tree_element() {
        let rows = cells(rows(&captured_snapshot()).expect("named VMs"));
        let vm_col = col(&columns(), "VM");
        let r = rows
            .iter()
            .find(|r| matches!(&r[vm_col], Cell::Text(n) if n == "vSAN File Service Node (1)"))
            .expect("the vSAN node's snapshot row");
        let at = |l: &str| r[col(&columns(), l)].clone();
        assert!(matches!(at("Name"), Cell::Text(ref s) if s == "eam-snapshot"));
        // `state` is the VM's power state at the moment the snapshot was taken,
        // not its state now: this VM is running, but the snapshot records
        // poweredOff. A hand-written fixture would very likely have mirrored
        // the VM's current state and quietly encoded the wrong meaning.
        assert!(matches!(at("State"), Cell::Text(ref s) if s == "poweredOff"));
        assert!(matches!(at("Date / time"), Cell::Text(_)));
    }
}

/// Nested snapshots, over the real capture rather than a hand-written tree.
#[cfg(test)]
mod nested_capture_tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_snapshot, cells, col};

    /// `snapshot.rootSnapshotList` is a `VirtualMachineSnapshotTree[]`, so its
    /// top-level elements carry the TYPE name — but a nested child repeats the
    /// FIELD name, `childSnapshotList`. That asymmetry is the single easiest
    /// thing to get wrong in vim25, and until this VM was built the lab had no
    /// nested snapshot to prove it against.
    #[test]
    fn a_parent_and_its_child_both_become_rows() {
        let rows = cells(rows(&captured_snapshot()).expect("named VMs"));
        let vm_col = col(&columns(), "VM");
        let name_col = col(&columns(), "Name");
        let mine: Vec<String> = rows
            .iter()
            .filter(|r| matches!(&r[vm_col], Cell::Text(n) if n == "invar-fixture-01"))
            .filter_map(|r| match &r[name_col] {
                Cell::Text(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(mine.len(), 2, "parent and child, got {mine:?}");
        assert!(mine.contains(&"invar-parent".to_string()));
        assert!(mine.contains(&"invar-child".to_string()));
    }

    /// Depth-first: a child is emitted immediately after its parent, not
    /// appended after every root.
    #[test]
    fn the_child_follows_its_parent() {
        let rows = cells(rows(&captured_snapshot()).expect("named VMs"));
        let name_col = col(&columns(), "Name");
        let names: Vec<String> = rows
            .iter()
            .filter_map(|r| match &r[name_col] {
                Cell::Text(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        let p = names.iter().position(|n| n == "invar-parent").expect("parent row");
        let c = names.iter().position(|n| n == "invar-child").expect("child row");
        assert_eq!(c, p + 1, "child should directly follow its parent: {names:?}");
    }
}
