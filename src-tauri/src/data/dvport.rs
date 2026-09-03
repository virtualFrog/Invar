//! dvPort — one row per distributed port group.
//!
//! Every setting here is wrapped in an inheritance envelope: a distributed
//! switch stores each policy as an object carrying `inherited` plus the
//! effective `value`, and a port group either overrides it or inherits the
//! switch's. Reading the field directly instead of its `value` child yields an
//! empty cell and no error, which is why `hostnet::policy_value` exists and why
//! this was checked against a real response first.
//!
//! `vlan` is the exception: it holds `vlanId` directly rather than a wrapped
//! value, because the field is polymorphic — a trunk port group carries VLAN
//! *ranges* under a different type instead. Only single-VLAN groups exist in
//! the reference lab, so trunk and private-VLAN forms are not implemented.

use super::hostnet::{policy_bool, policy_num, policy_value, walk};
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

/// `DistributedVirtualPortgroup` properties this sheet reads.
pub const DVPG_PROPS: &[&str] = &["config"];

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("Port"),
        Column::text("Switch"),
        Column::text("Type"),
        Column::number("# Ports"),
        Column::number("VLAN"),
        Column::bool("Blocked"),
        Column::bool("Allow Promiscuous"),
        Column::bool("Mac Changes"),
        Column::bool("Forged Transmits"),
        Column::text("Policy"),
        Column::bool("Reverse Policy"),
        Column::bool("Notify Switch"),
        Column::bool("Rolling Order"),
        Column::text("Active Uplink"),
        Column::text("Standby Uplink"),
        Column::bool("Check Beacon"),
        Column::bool("Check Duplex"),
        Column::text("Check Speed"),
        Column::number("Speed"),
        Column::bool("In Traffic Shaping"),
        Column::number("In Avg"),
        Column::number("In Peak"),
        Column::number("In Burst"),
        Column::bool("Out Traffic Shaping"),
        Column::number("Out Avg"),
        Column::number("Out Peak"),
        Column::number("Out Burst"),
    ]
}

/// Join a repeating element's text into one cell, as RVTools does.
fn joined(root: Option<&crate::vcenter::xml::Element>, path: &str, field: &str) -> Option<String> {
    let node = root.and_then(|r| walk(r, path))?;
    let joined: Vec<String> = node
        .children_named(field)
        .map(|e| e.text.clone())
        .filter(|s| !s.is_empty())
        .collect();
    (!joined.is_empty()).then(|| joined.join(", "))
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let mut rows = Vec::new();

    for pg in &snap.dvportgroups {
        let config = pg.prop("config");
        let dpc = config.and_then(|c| c.child("defaultPortConfig"));

        // The switch is a moref; the inventory index turns it into a name.
        let switch = config
            .and_then(|c| c.child("distributedVirtualSwitch"))
            .map(|e| e.text.clone())
            .filter(|s| !s.is_empty())
            .map(|m| snap.paths.name_of(&m).unwrap_or(m));

        rows.push((
            pg.moref.clone(),
            vec![
                Cell::opt_text(config.and_then(|c| c.text_at("name")).filter(|s| !s.is_empty())),
                Cell::opt_text(switch),
                // earlyBinding / lateBinding / ephemeral.
                Cell::opt_text(config.and_then(|c| c.text_at("type")).filter(|s| !s.is_empty())),
                Cell::opt_num(
                    config.and_then(|c| c.text_at("numPorts")).and_then(|v| v.parse::<f64>().ok()),
                ),
                // Not wrapped: vlan holds vlanId directly.
                Cell::opt_num(
                    dpc.and_then(|d| d.text_at("vlan/vlanId")).and_then(|v| v.parse::<f64>().ok()),
                ),
                Cell::opt_bool(policy_bool(dpc, "blocked")),
                Cell::opt_bool(policy_bool(dpc, "securityPolicy/allowPromiscuous")),
                Cell::opt_bool(policy_bool(dpc, "securityPolicy/macChanges")),
                Cell::opt_bool(policy_bool(dpc, "securityPolicy/forgedTransmits")),
                Cell::opt_text(policy_value(dpc, "uplinkTeamingPolicy/policy")),
                Cell::opt_bool(policy_bool(dpc, "uplinkTeamingPolicy/reversePolicy")),
                Cell::opt_bool(policy_bool(dpc, "uplinkTeamingPolicy/notifySwitches")),
                Cell::opt_bool(policy_bool(dpc, "uplinkTeamingPolicy/rollingOrder")),
                Cell::opt_text(joined(
                    dpc,
                    "uplinkTeamingPolicy/uplinkPortOrder",
                    "activeUplinkPort",
                )),
                Cell::opt_text(joined(
                    dpc,
                    "uplinkTeamingPolicy/uplinkPortOrder",
                    "standbyUplinkPort",
                )),
                Cell::opt_bool(policy_bool(dpc, "uplinkTeamingPolicy/failureCriteria/checkBeacon")),
                Cell::opt_bool(policy_bool(dpc, "uplinkTeamingPolicy/failureCriteria/checkDuplex")),
                Cell::opt_text(policy_value(dpc, "uplinkTeamingPolicy/failureCriteria/checkSpeed")),
                Cell::opt_num(policy_num(dpc, "uplinkTeamingPolicy/failureCriteria/speed")),
                Cell::opt_bool(policy_bool(dpc, "inShapingPolicy/enabled")),
                Cell::opt_num(policy_num(dpc, "inShapingPolicy/averageBandwidth")),
                Cell::opt_num(policy_num(dpc, "inShapingPolicy/peakBandwidth")),
                Cell::opt_num(policy_num(dpc, "inShapingPolicy/burstSize")),
                Cell::opt_bool(policy_bool(dpc, "outShapingPolicy/enabled")),
                Cell::opt_num(policy_num(dpc, "outShapingPolicy/averageBandwidth")),
                Cell::opt_num(policy_num(dpc, "outShapingPolicy/peakBandwidth")),
                Cell::opt_num(policy_num(dpc, "outShapingPolicy/burstSize")),
            ],
        ));
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "dvPort",
    columns,
    vm_props: &[],
    host_props: &[],
    dvs_props: &[],
    dvpg_props: &[DVPG_PROPS],
    // RVTools' dvPort carries no Datacenter or Cluster column.
    source: RowSource::None,
    rows,
};

pub async fn fetch_dvport_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{
        captured_dvportgroups, captured_snapshot, cells, col,
    };

    fn at(row: &[Cell], label: &str) -> Cell {
        row[col(&columns(), label)].clone()
    }

    fn snapshot() -> InventorySnapshot {
        captured_snapshot().with_distributed(Vec::new(), captured_dvportgroups())
    }

    #[test]
    fn one_row_per_port_group() {
        let snap = snapshot();
        let rows = cells(rows(&snap).expect("rows build"));
        assert_eq!(rows.len(), snap.dvportgroups.len());
        assert!(!rows.is_empty());
    }

    /// Every policy is stored as `{inherited, value}`. Reading the field itself
    /// rather than its `value` child gives an empty cell and no error, so this
    /// asserts the values actually came through.
    #[test]
    fn wrapped_policy_values_are_unwrapped() {
        let rows = cells(rows(&snapshot()).expect("rows build"));
        let r = &rows[0];
        assert!(matches!(at(r, "Allow Promiscuous"), Cell::Bool(_)));
        assert!(matches!(at(r, "Mac Changes"), Cell::Bool(_)));
        assert!(matches!(at(r, "Policy"), Cell::Text(ref s) if s.starts_with("loadbalance")));
        assert!(matches!(at(r, "In Avg"), Cell::Number(_)));
    }

    /// `vlan` is the exception to the envelope: it carries `vlanId` directly.
    #[test]
    fn vlan_is_read_directly_not_through_a_value_wrapper() {
        let rows = cells(rows(&snapshot()).expect("rows build"));
        assert!(
            rows.iter().any(|r| matches!(at(r, "VLAN"), Cell::Number(_))),
            "at least one port group should report a VLAN id"
        );
    }

    /// The switch is stored as a moref and must be resolved to a name.
    #[test]
    fn the_switch_is_resolved_to_a_name() {
        let rows = cells(rows(&snapshot()).expect("rows build"));
        for r in &rows {
            if let Cell::Text(s) = at(r, "Switch") {
                assert!(!s.starts_with("dvs-"), "switch should be a name, got {s:?}");
            }
        }
    }

    /// Uplink order is a repeating element, joined into one cell the way
    /// RVTools shows it.
    #[test]
    fn active_uplinks_are_joined_into_one_cell() {
        let rows = cells(rows(&snapshot()).expect("rows build"));
        assert!(
            rows.iter().any(|r| matches!(at(r, "Active Uplink"), Cell::Text(ref s) if s.contains("uplink"))),
            "uplink names should come through"
        );
    }
}
