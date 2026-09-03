//! vFileInfo — one row per file on a datastore.
//!
//! # This sheet is opt-in, and that is not a style choice
//!
//! Every other sheet reads inventory the vCenter already holds. This one walks
//! datastore filesystems through `HostDatastoreBrowser` and
//! `SearchDatastoreSubFolders_Task`, and the cost is in a different class.
//! Measured against the lab:
//!
//! | Datastore | Size | Result |
//! |---|---|---|
//! | 3 × VMFS | 1.3 TiB each | ~1.1s each, 136 files total |
//! | vSAN | 32.7 TiB, 164 VMs | **did not finish in 10 minutes** |
//!
//! So it is excluded from the export and runs only when someone opens this
//! sheet. Each datastore gets fifteen seconds; one that overruns is named in a
//! warning on this sheet and the others are still shown. Waiting longer does not
//! help — the vSAN store above would not have finished at any budget anyone
//! would sit through — so the sheet answers quickly with an honest gap rather
//! than blocking to reach the same gap slowly.
//!
//! The search returns one result per folder, each carrying `folderPath` and a
//! repeating `file` child — `file` is a field name inside a data object, not a
//! top-level array, so it is not the type name here.

use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

/// `Datastore` properties this sheet reads. `browser` is the handle the file
/// walk needs; it is not useful for anything else.
pub const DATASTORE_PROPS: &[&str] = &["name", "browser"];

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("Friendly Path Name"),
        Column::text("File Name"),
        Column::text("File Type"),
        Column::number("File Size in bytes"),
        Column::text("Modification"),
        Column::text("Path"),
    ]
}

/// `FloppyImageFileInfo` -> `Floppy image`, so the column reads as a file type
/// rather than as a class name.
fn friendly_type(xsi_type: Option<&str>) -> Option<String> {
    let t = xsi_type?;
    let base = t.strip_suffix("FileInfo").unwrap_or(t);
    if base.is_empty() {
        return Some(t.to_string());
    }
    // Split the CamelCase class into words: VmDiskFileInfo -> "Vm Disk".
    let mut out = String::new();
    for (i, c) in base.char_indices() {
        if i > 0 && c.is_uppercase() {
            out.push(' ');
        }
        out.push(c);
    }
    Some(out)
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let mut rows = Vec::new();

    for folder in &snap.datastore_files {
        let folder_path = folder.text_at("folderPath").unwrap_or_default();
        let moref = folder
            .child("datastore")
            .map(|d| d.text.clone())
            .unwrap_or_default();

        for file in folder.children_named("file") {
            let name = file.text_at("path").unwrap_or_default();
            rows.push((
                moref.clone(),
                vec![
                    Cell::Text(folder_path.clone()),
                    Cell::Text(name.clone()),
                    // The concrete class says what kind of file it is; the
                    // element name is the same for every entry.
                    Cell::opt_text(friendly_type(file.xsi_type.as_deref())),
                    Cell::opt_num(
                        file.text_at("fileSize").and_then(|v| v.parse::<f64>().ok()),
                    ),
                    Cell::opt_text(file.text_at("modification").filter(|s| !s.is_empty())),
                    // The full datastore path, which is what anyone would paste
                    // back into a command.
                    Cell::Text(format!("{folder_path}{name}")),
                ],
            ));
        }
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vFileInfo",
    columns,
    vm_props: &[],
    host_props: &[],
    dvs_props: &[],
    dvpg_props: &[],
    cluster_props: &[],
    datastore_props: &[DATASTORE_PROPS],
    rp_props: &[],
    wants_licenses: false,
    wants_about: false,
    wants_files: true,
    // The rows describe files, not an inventory object, so no location columns.
    source: RowSource::None,
    rows,
};

pub async fn fetch_vfileinfo_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_files, captured_snapshot, cells, col};

    fn snapshot() -> InventorySnapshot {
        captured_snapshot().with_datastore_files(captured_files())
    }

    fn at(row: &[Cell], label: &str) -> Cell {
        row[col(&columns(), label)].clone()
    }

    /// One row per file, not per folder. The search returns folders, each
    /// carrying its files as repeated `file` children.
    #[test]
    fn one_row_per_file() {
        let snap = snapshot();
        let expected: usize = snap
            .datastore_files
            .iter()
            .map(|f| f.children_named("file").count())
            .sum();
        let rows = cells(rows(&snap).expect("rows build"));
        assert_eq!(rows.len(), expected);
        assert!(expected > 0, "the capture should contain files");
    }

    /// `Path` is the folder and the file name joined, because that is the form
    /// anyone would paste back into a command.
    #[test]
    fn the_full_path_is_folder_plus_name() {
        let rows = cells(rows(&snapshot()).expect("rows build"));
        let r = &rows[0];
        if let (Cell::Text(folder), Cell::Text(name), Cell::Text(path)) =
            (at(r, "Friendly Path Name"), at(r, "File Name"), at(r, "Path"))
        {
            assert_eq!(path, format!("{folder}{name}"));
            assert!(path.starts_with('['), "a datastore path starts with [ds] — got {path:?}");
        } else {
            panic!("folder, name and path are all text");
        }
    }

    /// The file's kind is in its `xsi:type`, and is shown as words rather than
    /// as a vim25 class name.
    #[test]
    fn file_type_is_readable_rather_than_a_class_name() {
        assert_eq!(friendly_type(Some("VmDiskFileInfo")).as_deref(), Some("Vm Disk"));
        assert_eq!(friendly_type(Some("FolderFileInfo")).as_deref(), Some("Folder"));
        assert_eq!(friendly_type(None), None);
        let rows = cells(rows(&snapshot()).expect("rows build"));
        assert!(rows.iter().any(|r| matches!(at(r, "File Type"), Cell::Text(_))));
    }

    /// A snapshot that never ran the walk yields nothing, rather than an empty
    /// table that looks like a datastore with no files on it.
    #[test]
    fn without_a_walk_there_are_no_rows() {
        let rows = cells(rows(&captured_snapshot()).expect("rows build"));
        assert!(rows.is_empty());
    }
}
