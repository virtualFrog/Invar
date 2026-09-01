//! Multi-sheet .xlsx export in RVTools' format.
//!
//! The formatting here was read out of a real RVTools 4.6 export
//! (`reference/RVTools_export_all_*.xlsx`) rather than guessed:
//!
//! | Element        | RVTools                                                  |
//! |----------------|----------------------------------------------------------|
//! | Header row     | Verdana 9pt bold, white on solid black, left aligned     |
//! | Body text      | Verdana 9pt black, general alignment                     |
//! | Body integers  | Verdana 9pt, `#,##0`, right aligned                      |
//! | Body decimals  | Verdana 9pt, `#,##0.00`, right aligned                   |
//! | Dates          | real Excel serial dates, `yyyy/MM/dd HH:mm:ss`           |
//! | Booleans       | the text `True` / `False`, not Excel booleans            |
//! | Freeze panes   | `B2` — first row *and* first column frozen               |
//! | AutoFilter     | the whole range, header row included                     |
//! | Sheet order    | RVTools' own order, not the order sheets were fetched    |

use crate::data::{Cell, ColumnKind, Table};
use chrono::NaiveDateTime;
use rust_xlsxwriter::{Color, Format, FormatAlign, Workbook, Worksheet};

/// RVTools' sheet order. Sheets we don't produce are simply skipped, so the
/// ones we do produce still appear in the order RVTools would put them.
const RVTOOLS_SHEET_ORDER: &[&str] = &[
    "vInfo",
    "vCPU",
    "vMemory",
    "vDisk",
    "vPartition",
    "vNetwork",
    "vCD",
    "vUSB",
    "vSnapshot",
    "vTools",
    "vSource",
    "vRP",
    "vCluster",
    "vHost",
    "vHBA",
    "vNIC",
    "vSwitch",
    "vPort",
    "dvSwitch",
    "dvPort",
    "vSC_VMK",
    "vDatastore",
    "vMultiPath",
    "vLicense",
    "vFileInfo",
    "vHealth",
    "vMetaData",
];

struct Formats {
    header: Format,
    text: Format,
    integer: Format,
    decimal: Format,
    date: Format,
}

impl Formats {
    fn new() -> Self {
        let base = || Format::new().set_font_name("Verdana").set_font_size(9);
        Self {
            header: base()
                .set_bold()
                .set_font_color(Color::White)
                .set_background_color(Color::Black)
                .set_align(FormatAlign::Left),
            text: base(),
            integer: base().set_num_format("#,##0").set_align(FormatAlign::Right),
            decimal: base().set_num_format("#,##0.00").set_align(FormatAlign::Right),
            date: base().set_num_format("yyyy/MM/dd HH:mm:ss"),
        }
    }
}

/// vCenter returns RFC 3339 timestamps; RVTools stores real Excel dates.
///
/// Values are written as they come back from vCenter (UTC) — converting to the
/// local timezone would silently relabel every timestamp in the export.
fn parse_timestamp(s: &str) -> Option<NaiveDateTime> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.naive_utc())
}

/// How a column's body cells should be written.
enum ColumnFormat {
    Text,
    Integer,
    Decimal,
    Date,
}

/// RVTools styles a column uniformly, so the format is decided from the whole
/// column rather than cell by cell.
fn column_format(table: &Table, index: usize) -> ColumnFormat {
    let values = || table.rows.iter().filter_map(|r| r.get(index));

    match table.columns[index].kind {
        ColumnKind::Number => {
            let all_integral = values().all(|c| match c {
                Cell::Number(n) => n.fract() == 0.0,
                _ => true,
            });
            if all_integral {
                ColumnFormat::Integer
            } else {
                ColumnFormat::Decimal
            }
        }
        ColumnKind::Bool => ColumnFormat::Text,
        ColumnKind::Text => {
            let mut seen = false;
            let all_dates = values().all(|c| match c {
                Cell::Text(s) if !s.is_empty() => {
                    seen = true;
                    parse_timestamp(s).is_some()
                }
                _ => true,
            });
            if seen && all_dates {
                ColumnFormat::Date
            } else {
                ColumnFormat::Text
            }
        }
    }
}

fn write_sheet(sheet: &mut Worksheet, table: &Table, f: &Formats) -> Result<(), String> {
    sheet
        .set_name(&table.name)
        .map_err(|e| format!("could not name sheet {}: {e}", table.name))?;

    for (col, column) in table.columns.iter().enumerate() {
        sheet
            .write_string_with_format(0, col as u16, &column.label, &f.header)
            .map_err(|e| format!("{}: header {}: {e}", table.name, column.label))?;
    }

    let formats: Vec<ColumnFormat> = (0..table.columns.len())
        .map(|i| column_format(table, i))
        .collect();

    for (r, row) in table.rows.iter().enumerate() {
        let excel_row = r as u32 + 1;
        for (c, cell) in row.iter().enumerate() {
            let col = c as u16;
            let err = |e: rust_xlsxwriter::XlsxError| {
                format!("{}: row {} col {}: {e}", table.name, excel_row + 1, col + 1)
            };
            match cell {
                // An empty cell still gets the column's format, the way RVTools
                // writes styled-but-blank cells.
                Cell::Empty => sheet
                    .write_blank(excel_row, col, &f.text)
                    .map_err(err)
                    .map(|_| ())?,
                // RVTools writes booleans as the words True/False, not as
                // Excel booleans — filters and lookups depend on that.
                Cell::Bool(b) => sheet
                    .write_string_with_format(excel_row, col, if *b { "True" } else { "False" }, &f.text)
                    .map_err(err)
                    .map(|_| ())?,
                Cell::Number(n) => {
                    let fmt = match formats[c] {
                        ColumnFormat::Integer => &f.integer,
                        _ => &f.decimal,
                    };
                    sheet.write_number_with_format(excel_row, col, *n, fmt).map_err(err).map(|_| ())?
                }
                Cell::Text(s) => match (&formats[c], parse_timestamp(s)) {
                    (ColumnFormat::Date, Some(dt)) => sheet
                        .write_datetime_with_format(excel_row, col, &dt, &f.date)
                        .map_err(err)
                        .map(|_| ())?,
                    _ => sheet
                        .write_string_with_format(excel_row, col, s, &f.text)
                        .map_err(err)
                        .map(|_| ())?,
                },
            }
        }
    }

    // Freeze the header row and the first column, as RVTools does (pane at B2).
    sheet.set_freeze_panes(1, 1).map_err(|e| format!("{}: freeze panes: {e}", table.name))?;

    // AutoFilter covers the header row plus every data row. With no data rows
    // the filter still spans the header, matching RVTools' empty sheets.
    let last_row = table.rows.len() as u32;
    let last_col = table.columns.len().saturating_sub(1) as u16;
    sheet
        .autofilter(0, 0, last_row, last_col)
        .map_err(|e| format!("{}: autofilter: {e}", table.name))?;

    sheet.autofit();
    Ok(())
}

/// The `vMetaData` sheet.
///
/// RVTools' column names are kept so anything that parses its exports still
/// works, but the values name *this* tool — claiming to be an RVTools version
/// would misstate where the data came from.
fn write_metadata(sheet: &mut Worksheet, servers: &[String], f: &Formats) -> Result<(), String> {
    let table = Table {
        name: "vMetaData".into(),
        columns: vec![
            crate::data::Column::text("RVTools major version"),
            crate::data::Column::text("RVTools version"),
            crate::data::Column::text("xlsx creation datetime"),
            crate::data::Column::text("Server"),
        ],
        rows: servers
            .iter()
            .map(|server| {
                vec![
                    Cell::Text(env!("CARGO_PKG_NAME").to_string()),
                    Cell::Text(format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))),
                    Cell::Text(chrono::Utc::now().to_rfc3339()),
                    Cell::Text(server.clone()),
                ]
            })
            .collect(),
        warnings: Vec::new(),
    };
    write_sheet(sheet, &table, f)
}

/// Write every table to one workbook, in RVTools' sheet order.
pub fn write_workbook(tables: &[Table], servers: &[String], path: &std::path::Path) -> Result<(), String> {
    let formats = Formats::new();
    let mut workbook = Workbook::new();

    let mut ordered: Vec<&Table> = Vec::new();
    for name in RVTOOLS_SHEET_ORDER {
        if let Some(t) = tables.iter().find(|t| t.name == *name) {
            ordered.push(t);
        }
    }
    // Anything not in RVTools' list still gets exported rather than dropped.
    for t in tables {
        if !RVTOOLS_SHEET_ORDER.contains(&t.name.as_str()) {
            ordered.push(t);
        }
    }

    for table in ordered {
        write_sheet(workbook.add_worksheet(), table, &formats)?;
    }
    write_metadata(workbook.add_worksheet(), servers, &formats)?;

    workbook
        .save(path)
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    Ok(())
}

/// RVTools' filename convention: `RVTools_export_all_YYYY-MM-DD_HH.mm.ss.xlsx`.
pub fn default_filename() -> String {
    format!(
        "RVTools_export_all_{}.xlsx",
        chrono::Local::now().format("%Y-%m-%d_%H.%M.%S")
    )
}
