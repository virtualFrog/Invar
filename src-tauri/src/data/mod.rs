//! Table model shared by every sheet.
//!
//! Each fetcher declares its columns once and emits rows against them. The
//! renderer and the xlsx writer both consume `Table`, so a cross-cutting column
//! like `VI SDK Server` is appended in two generic places rather than edited
//! into ~24 per-table definitions.

pub mod common;
pub mod vdisk;
pub mod vhealth;
pub mod vhost;
pub mod vinfo;
pub mod insights;
pub mod snapshot;
pub mod topology;
pub mod vsnapshot;

use serde::Serialize;
use snapshot::SheetSpec;

/// Every sheet the app knows about, in tab order.
///
/// This is the single registry: `list_sheets`, `fetch_sheet` and the export all
/// drive off it, so adding a sheet is one module plus one line here.
pub const SHEETS: &[&SheetSpec] = &[
    &vinfo::SPEC,
    &vhost::SPEC,
    &vdisk::SPEC,
    &vsnapshot::SPEC,
    &vhealth::SPEC,
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
}

/// The source-vCenter column RVTools puts on nearly every sheet. Appended
/// generically so no fetcher has to remember it.
pub const VI_SDK_SERVER: &str = "VI SDK Server";

impl Table {
    pub fn new(name: &str, columns: Vec<Column>) -> Self {
        Self { name: name.into(), columns, rows: Vec::new(), warnings: Vec::new() }
    }

    /// Append rows from one vCenter, tagging each with its source server.
    pub fn extend_from(&mut self, server: &str, rows: Vec<Vec<Cell>>) {
        for mut row in rows {
            row.push(Cell::Text(server.to_string()));
            self.rows.push(row);
        }
    }

    /// Call once after construction so `VI SDK Server` lines up with the value
    /// `extend_from` appends.
    pub fn with_source_column(mut self) -> Self {
        self.columns.push(Column::text(VI_SDK_SERVER));
        self
    }
}
