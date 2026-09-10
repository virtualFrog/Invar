//! Table model shared by every sheet.
//!
//! Each fetcher declares its columns once and emits rows against them. The
//! renderer and the xlsx writer both consume `Table`, so a cross-cutting column
//! like `VI SDK Server` is appended in two generic places rather than edited
//! into ~24 per-table definitions.

pub mod common;
pub mod drift;
pub mod dvport;
pub mod dvswitch;
pub mod hostnet;
pub mod insights;
pub mod snapshot;
pub mod topology;
pub mod vcd;
pub mod vcluster;
pub mod vcpu;
pub mod vdatastore;
pub mod vdisk;
pub mod vfileinfo;
pub mod vhba;
pub mod vhealth;
pub mod vhost;
pub mod vinfo;
pub mod vlicense;
pub mod vmemory;
pub mod vmultipath;
pub mod vnetwork;
pub mod vnic;
pub mod vpartition;
pub mod vport;
pub mod vrp;
pub mod vsc_vmk;
pub mod vsource;
pub mod vsnapshot;
pub mod vswitch;
pub mod vtools;
pub mod vusb;

use serde::Serialize;
use snapshot::SheetSpec;

/// Every sheet the app knows about, in tab order.
///
/// This is the single registry: `list_sheets`, `fetch_sheet` and the export all
/// drive off it, so adding a sheet is one module plus one line here.
/// RVTools' own sheet order, so tabs and the export line up with a real
/// RVTools workbook. `export.rs` re-orders anyway via `RVTOOLS_SHEET_ORDER`;
/// this is what the UI shows.
pub const SHEETS: &[&SheetSpec] = &[
    &vinfo::SPEC,
    &vcpu::SPEC,
    &vmemory::SPEC,
    &vdisk::SPEC,
    &vpartition::SPEC,
    &vnetwork::SPEC,
    &vcd::SPEC,
    &vusb::SPEC,
    &vsnapshot::SPEC,
    &vtools::SPEC,
    &vsource::SPEC,
    &vrp::SPEC,
    &vcluster::SPEC,
    &vhost::SPEC,
    &vhba::SPEC,
    &vnic::SPEC,
    &vswitch::SPEC,
    &vport::SPEC,
    &dvswitch::SPEC,
    &dvport::SPEC,
    &vsc_vmk::SPEC,
    &vdatastore::SPEC,
    &vmultipath::SPEC,
    &vfileinfo::SPEC,
    &vlicense::SPEC,
    &vhealth::SPEC,
    // Not an RVTools sheet, so it sits after the 26 that are. `export.rs`
    // appends anything absent from `RVTOOLS_SHEET_ORDER` rather than dropping
    // it, so the workbook carries it too.
    &drift::SPEC,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ColumnKind {
    Text,
    Number,
    Bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Column {
    /// RVTools' exact label, so exports match its sheets.
    pub label: String,
    pub kind: ColumnKind,
}

impl Column {
    pub fn text(label: &str) -> Self {
        Self { label: label.into(), kind: ColumnKind::Text }
    }
    pub fn number(label: &str) -> Self {
        Self { label: label.into(), kind: ColumnKind::Number }
    }
    pub fn bool(label: &str) -> Self {
        Self { label: label.into(), kind: ColumnKind::Bool }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Cell {
    Text(String),
    Number(f64),
    Bool(bool),
    Empty,
}

impl Cell {
    /// A missing value stays empty rather than becoming `0` or `"unknown"` —
    /// in an inventory, "not reported" and "zero" are different facts.
    pub fn opt_text(v: Option<String>) -> Self {
        v.map(Cell::Text).unwrap_or(Cell::Empty)
    }
    pub fn opt_num<T: Into<f64>>(v: Option<T>) -> Self {
        v.map(|n| Cell::Number(n.into())).unwrap_or(Cell::Empty)
    }
    pub fn opt_bool(v: Option<bool>) -> Self {
        v.map(Cell::Bool).unwrap_or(Cell::Empty)
    }
}

impl From<String> for Cell {
    fn from(s: String) -> Self {
        Cell::Text(s)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Table {
    /// RVTools sheet name, e.g. `vInfo`.
    pub name: String,
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Cell>>,
    /// Per-vCenter failures that did not stop the rest of the fetch. Shown in
    /// the UI: a short list that looks complete is the worst outcome for an
    /// inventory tool.
    #[serde(default)]
    pub warnings: Vec<String>,
    /// What zero rows means on this sheet, when it does not mean "no data".
    /// `None` leaves the UI's generic wording in place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty_note: Option<String>,
}

/// What an empty sheet means, for the sheets where no rows is the *good*
/// result.
///
/// Most sheets enumerate objects, so zero rows means the vCenter held none of
/// them. A findings sheet inverts that: zero rows is a clean bill of health,
/// and telling the reader "the vCenter reported no objects of this kind" turns
/// a verified-healthy result into something that reads like a broken query.
///
/// Kept here beside [`SHEETS`] rather than as a `SheetSpec` field so that
/// adding one does not mean editing all 26 existing specs — the same reason
/// `VI SDK Server` is appended generically rather than restated per sheet.
fn empty_note_for(sheet: &str) -> Option<&'static str> {
    match sheet {
        "Cluster Drift" => Some(
            "No drift found. Every host agrees with the rest of its cluster on all \
             the settings compared, which is the result you want. Clusters of one \
             host, and settings no host reported, are not compared — see the sheet's \
             notes.",
        ),
        "vHealth" => Some(
            "No health findings. RVTools' checks all passed against this inventory.",
        ),
        _ => None,
    }
}

/// The source-vCenter column RVTools puts on nearly every sheet. Appended
/// generically so no fetcher has to remember it.
pub const VI_SDK_SERVER: &str = "VI SDK Server";

/// The source vCenter's instance UUID, RVTools' companion to `VI SDK Server`
/// and appended the same way.
///
/// A hostname is not a stable identity — a vCenter can be renamed, and two
/// estates can each have a `vcenter01`. The instance UUID is what says two
/// exports came from the same server. Red Hat's Migration Advisor rejects an
/// RVTools upload that lacks it.
pub const VI_SDK_UUID: &str = "VI SDK UUID";

/// Where an object sits in the inventory. RVTools carries these on nearly every
/// sheet; like `VI SDK Server` they are appended in one place rather than
/// restated by ~20 sheet modules.
pub const DATACENTER: &str = "Datacenter";
pub const CLUSTER: &str = "Cluster";
pub const FOLDER: &str = "Folder";

impl Table {
    pub fn new(name: &str, columns: Vec<Column>) -> Self {
        Self {
            name: name.into(),
            columns,
            rows: Vec::new(),
            warnings: Vec::new(),
            empty_note: empty_note_for(name).map(str::to_string),
        }
    }

    /// Append rows from one vCenter, tagging each with its inventory location
    /// and its source server.
    ///
    /// `rows` carries the moref of the object each row describes, so the
    /// location columns can be resolved here rather than in every sheet.
    pub fn extend_from(
        &mut self,
        server: &str,
        instance_uuid: Option<&str>,
        rows: Vec<(String, Vec<Cell>)>,
        source: snapshot::RowSource,
        paths: &snapshot::PathIndex,
    ) {
        for (moref, mut row) in rows {
            match source {
                snapshot::RowSource::Vm => {
                    row.push(Cell::opt_text(paths.datacenter_of(&moref)));
                    row.push(Cell::opt_text(paths.cluster_of_vm(&moref)));
                    row.push(Cell::opt_text(paths.folder_of(&moref)));
                }
                snapshot::RowSource::Host => {
                    row.push(Cell::opt_text(paths.datacenter_of(&moref)));
                    row.push(Cell::opt_text(paths.cluster_of_host(&moref)));
                }
                snapshot::RowSource::None => {}
            }
            row.push(Cell::Text(server.to_string()));
            row.push(Cell::opt_text(instance_uuid.map(str::to_string)));
            self.rows.push(row);
        }
    }

    /// Call once after construction so the appended columns line up with the
    /// values `extend_from` pushes. Order matters and mirrors RVTools: the
    /// location columns sit before `VI SDK Server`, which is always last.
    ///
    /// vHost carries `Datacenter` and `Cluster` but no `Folder` — a host's
    /// folder is the datacenter's `host` folder, which RVTools does not show.
    /// vHealth carries none of them: its sheet is three columns wide.
    pub fn with_location_columns(mut self, source: snapshot::RowSource) -> Self {
        match source {
            snapshot::RowSource::Vm => {
                self.columns.push(Column::text(DATACENTER));
                self.columns.push(Column::text(CLUSTER));
                self.columns.push(Column::text(FOLDER));
            }
            snapshot::RowSource::Host => {
                self.columns.push(Column::text(DATACENTER));
                self.columns.push(Column::text(CLUSTER));
            }
            snapshot::RowSource::None => {}
        }
        self
    }

    /// Call once after construction so `VI SDK Server` lines up with the value
    /// `extend_from` appends.
    pub fn with_source_column(mut self) -> Self {
        self.columns.push(Column::text(VI_SDK_SERVER));
        self.columns.push(Column::text(VI_SDK_UUID));
        self
    }
}
