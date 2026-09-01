//! vSnapshot — one row per snapshot.
//!
//! Sourced from `snapshot.rootSnapshotList`, whose elements are
//! `<VirtualMachineSnapshotTree>`. Sizes come from `layoutEx`: each snapshot's
//! `dataKey` and `memoryKey` index into `layoutEx.file`.

use super::common::{bytes_to_mib, host_names, VmContext, VM_CONTEXT_PROPS};
use super::{Cell, Column, Table};
use crate::vcenter::soap::ManagedObject;
use crate::vcenter::xml::Element;
use crate::vcenter::{Session, VCenterConnection};
use std::collections::HashMap;

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

pub async fn fetch_vsnapshot_core(session: &Session) -> Result<Vec<Vec<Cell>>, String> {
    let hosts = host_names(session).await?;
    let mut props = VM_CONTEXT_PROPS.to_vec();
    props.extend_from_slice(&[
        "snapshot.rootSnapshotList",
        "snapshot.currentSnapshot",
        "layoutEx.file",
        "layoutEx.snapshot",
    ]);
    let vms = session.soap.retrieve("VirtualMachine", &props).await?;

    let mut rows = Vec::new();
    for vm in vms {
        let Some(ctx) = VmContext::from(&vm, &hosts)? else {
            continue;
        };

        let mut snapshots = Vec::new();
        for root in vm.array_prop("snapshot.rootSnapshotList") {
            flatten(root, &mut snapshots);
        }
        if snapshots.is_empty() {
            continue;
        }

        let files = file_layout(&vm);
        let layout = snapshot_layout(&vm);
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

            rows.push(vec![
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
            ]);
        }
    }

    Ok(rows)
}

pub async fn fetch_vsnapshot_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    let mut table = Table::new("vSnapshot", columns()).with_source_column();
    for conn in conns {
        let label = conn.label();
        match cache.get(conn).await {
            Ok(session) => match fetch_vsnapshot_core(&session).await {
                Ok(rows) => table.extend_from(&label, rows),
                Err(e) => table.warnings.push(format!("{label}: {e}")),
            },
            Err(e) => table.warnings.push(format!("{label}: {e}")),
        }
    }
    table
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
