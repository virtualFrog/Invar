//! vDisk — one row per virtual disk.
//!
//! Sourced from `config.hardware.device`, filtered to `xsi:type="VirtualDisk"`.
//! The array elements are named `<VirtualDevice>` after the field's declared
//! type; the concrete device type is only in `xsi:type`, so filtering on the
//! element name would silently yield nothing.

use super::common::{host_names, VmContext, VM_CONTEXT_PROPS};
use super::{Cell, Column, Table};
use crate::vcenter::xml::Element;
use crate::vcenter::{Session, VCenterConnection};
use std::collections::HashMap;

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("VM"),
        Column::text("Powerstate"),
        Column::bool("Template"),
        Column::text("Disk"),
        Column::number("Disk Key"),
        Column::text("Disk UUID"),
        Column::text("Disk Path"),
        Column::number("Capacity MiB"),
        Column::bool("Raw"),
        Column::text("Disk Mode"),
        Column::text("Sharing mode"),
        Column::bool("Thin"),
        Column::bool("Eagerly Scrub"),
        Column::bool("Split"),
        Column::bool("Write Through"),
        Column::text("Level"),
        Column::number("Shares"),
        Column::number("Reservation"),
        Column::number("Limit"),
        Column::text("Controller"),
        Column::number("Unit #"),
        Column::text("Raw LUN ID"),
        Column::text("Raw Comp. Mode"),
        Column::text("Host"),
        Column::text("Annotation"),
    ]
}

fn text(el: &Element, path: &str) -> Option<String> {
    el.text_at(path).filter(|s| !s.is_empty())
}

fn number(el: &Element, path: &str) -> Option<f64> {
    text(el, path)?.parse().ok()
}

fn boolean(el: &Element, path: &str) -> Option<bool> {
    match text(el, path)?.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Device key → label, so a disk's `controllerKey` can name its controller
/// ("SCSI controller 0").
fn controller_labels(devices: &[&Element]) -> HashMap<String, String> {
    devices
        .iter()
        .filter_map(|d| Some((d.text_at("key")?, d.text_at("deviceInfo/label")?)))
        .collect()
}

pub async fn fetch_vdisk_core(session: &Session) -> Result<Vec<Vec<Cell>>, String> {
    let hosts = host_names(session).await?;
    let mut props = VM_CONTEXT_PROPS.to_vec();
    props.push("config.hardware.device");
    let vms = session.soap.retrieve("VirtualMachine", &props).await?;

    let mut rows = Vec::new();
    for vm in vms {
        let Some(ctx) = VmContext::from(&vm, &hosts)? else {
            continue;
        };
        let devices = vm.array_prop("config.hardware.device");
        let controllers = controller_labels(&devices);

        for disk in devices
            .iter()
            .filter(|d| d.xsi_type.as_deref() == Some("VirtualDisk"))
        {
            let backing = disk.child("backing");
            // A raw device mapping is backed by RawDiskMappingVer1BackingInfo
            // rather than a flat vmdk. None exist in the lab used for
            // development, so these columns are expected to be empty there.
            let is_raw = backing
                .and_then(|b| b.xsi_type.as_deref())
                .is_some_and(|t| t.starts_with("RawDiskMapping"));

            rows.push(vec![
                Cell::Text(ctx.name.clone()),
                Cell::opt_text(ctx.power_state.clone()),
                Cell::opt_bool(ctx.template),
                Cell::opt_text(text(disk, "deviceInfo/label")),
                Cell::opt_num(number(disk, "key")),
                Cell::opt_text(backing.and_then(|b| text(b, "uuid"))),
                Cell::opt_text(backing.and_then(|b| text(b, "fileName"))),
                // capacityInKB is KiB; RVTools' column is MiB.
                Cell::opt_num(number(disk, "capacityInKB").map(|kb| (kb / 1024.0 * 100.0).round() / 100.0)),
                Cell::Bool(is_raw),
                Cell::opt_text(backing.and_then(|b| text(b, "diskMode"))),
                Cell::opt_text(backing.and_then(|b| text(b, "sharing"))),
                Cell::opt_bool(backing.and_then(|b| boolean(b, "thinProvisioned"))),
                Cell::opt_bool(backing.and_then(|b| boolean(b, "eagerlyScrub"))),
                Cell::opt_bool(backing.and_then(|b| boolean(b, "split"))),
                Cell::opt_bool(backing.and_then(|b| boolean(b, "writeThrough"))),
                // Level/Shares/Reservation/Limit are the disk's storage I/O
                // allocation, not the device-level <shares> block above it.
                Cell::opt_text(text(disk, "storageIOAllocation/shares/level")),
                Cell::opt_num(number(disk, "storageIOAllocation/shares/shares")),
                Cell::opt_num(number(disk, "storageIOAllocation/reservation")),
                Cell::opt_num(number(disk, "storageIOAllocation/limit")),
                Cell::opt_text(
                    text(disk, "controllerKey").and_then(|k| controllers.get(&k).cloned()),
                ),
                Cell::opt_num(number(disk, "unitNumber")),
                Cell::opt_text(backing.and_then(|b| text(b, "lunUuid"))),
                Cell::opt_text(backing.and_then(|b| text(b, "compatibilityMode"))),
                Cell::opt_text(ctx.host.clone()),
                Cell::opt_text(ctx.annotation.clone()),
            ]);
        }
    }

    Ok(rows)
}

pub async fn fetch_vdisk_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    let mut table = Table::new("vDisk", columns()).with_source_column();
    for conn in conns {
        let label = conn.label();
        match cache.get(conn).await {
            Ok(session) => match fetch_vdisk_core(&session).await {
                Ok(rows) => table.extend_from(&label, rows),
                Err(e) => table.warnings.push(format!("{label}: {e}")),
            },
            Err(e) => table.warnings.push(format!("{label}: {e}")),
        }
    }
    table
}
