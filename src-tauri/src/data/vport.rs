//! vPort — one row per **standard** port group on a host.
//!
//! Reads `config.network.portgroup` off the shared host snapshot.
//!
//! Policy comes from `computedPolicy` rather than `spec/policy`: the former is
//! the effective configuration, the latter only what was explicitly set on the
//! group. A port group that inherits its teaming from the switch has no
//! `nicTeaming` under `spec/policy` at all.
//!
//! The lab is otherwise entirely distributed-switched, so a standard switch and
//! port group were created on one host purely so this sheet has something real
//! to parse (`sttools-vSwitch` / `sttools-pg`, see `docs/LAB-ENVIRONMENT.md`).
//! Distributed port groups are a different sheet, `dvPort`.

use super::hostnet::HOST_NET_PROPS;
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

pub const HOST_PROPS: &[&str] = HOST_NET_PROPS;

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("Host"),
        Column::text("Port Group"),
        Column::text("Switch"),
        Column::number("VLAN"),
        Column::bool("Promiscuous Mode"),
        Column::bool("Mac Changes"),
        Column::bool("Forged Transmits"),
        Column::bool("Traffic Shaping"),
        Column::number("Width"),
        Column::number("Peak"),
        Column::number("Burst"),
        Column::text("Policy"),
        Column::bool("Reverse Policy"),
        Column::bool("Notify Switch"),
        Column::bool("Rolling Order"),
    ]
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let mut rows = Vec::new();

    for host in &snap.hosts {
        let Some(name) = host.str_prop("name") else {
            return Err(format!("HostSystem {} returned no name property", host.moref));
        };

        for pg in host.array_prop("config.network.portgroup") {
            let spec = pg.child("spec");
            // `computedPolicy` is what actually applies: the port group's own
            // settings merged with what it inherits from the switch.
            // `spec/policy` holds only what was explicitly set, so teaming that
            // came from the switch would read as empty. Verified against a real
            // port group, whose spec carries security but no nicTeaming while
            // computedPolicy carries both.
            let policy = pg
                .child("computedPolicy")
                .or_else(|| spec.and_then(|s| s.child("policy")));
            let b = |p: &str| {
                policy.and_then(|x| x.text_at(p)).map(|v| v == "true")
            };
            let n = |p: &str| {
                policy.and_then(|x| x.text_at(p)).and_then(|v| v.parse::<f64>().ok())
            };

            rows.push((
                host.moref.clone(),
                vec![
                    Cell::Text(name.clone()),
                    Cell::opt_text(spec.and_then(|s| s.text_at("name")).filter(|s| !s.is_empty())),
                    Cell::opt_text(
                        spec.and_then(|s| s.text_at("vswitchName")).filter(|s| !s.is_empty()),
                    ),
                    Cell::opt_num(
                        spec.and_then(|s| s.text_at("vlanId")).and_then(|v| v.parse::<f64>().ok()),
                    ),
                    Cell::opt_bool(b("security/allowPromiscuous")),
                    Cell::opt_bool(b("security/macChanges")),
                    Cell::opt_bool(b("security/forgedTransmits")),
                    Cell::opt_bool(b("shapingPolicy/enabled")),
                    Cell::opt_num(n("shapingPolicy/averageBandwidth")),
                    Cell::opt_num(n("shapingPolicy/peakBandwidth")),
                    Cell::opt_num(n("shapingPolicy/burstSize")),
                    Cell::opt_text(policy.and_then(|p| p.text_at("nicTeaming/policy"))),
                    Cell::opt_bool(b("nicTeaming/reversePolicy")),
                    Cell::opt_bool(b("nicTeaming/notifySwitches")),
                    Cell::opt_bool(b("nicTeaming/rollingOrder")),
                ],
            ));
        }
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vPort",
    columns,
    vm_props: &[],
    host_props: &[HOST_PROPS],
    dvs_props: &[],
    dvpg_props: &[],
    source: RowSource::Host,
    rows,
};

pub async fn fetch_vport_all(
    conns: &[VCenterConnection],
    cache: &crate::vcenter::SessionCache,
) -> Table {
    super::snapshot::fetch_table(&SPEC, conns, cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::snapshot::test_support::{captured_snapshot, cells, col};

    fn at(row: &[Cell], label: &str) -> Cell {
        row[col(&columns(), label)].clone()
    }

    #[test]
    fn one_row_per_standard_port_group() {
        let rows = cells(rows(&captured_snapshot()).expect("named host"));
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert!(matches!(at(r, "Port Group"), Cell::Text(ref s) if s == "sttools-pg"));
        assert!(matches!(at(r, "Switch"), Cell::Text(ref s) if s == "sttools-vSwitch"));
        assert!(matches!(at(r, "VLAN"), Cell::Number(n) if n == 101.0));
    }

    /// The group's own security settings come through.
    #[test]
    fn explicit_settings_are_reported() {
        let rows = cells(rows(&captured_snapshot()).expect("named host"));
        let r = &rows[0];
        assert!(matches!(at(r, "Promiscuous Mode"), Cell::Bool(true)));
        assert!(matches!(at(r, "Mac Changes"), Cell::Bool(false)));
        assert!(matches!(at(r, "Forged Transmits"), Cell::Bool(true)));
    }

    /// Teaming was never set on this port group -- it inherits the switch's. It
    /// appears in `computedPolicy` and **not** in `spec/policy`, so reading the
    /// spec would leave the column empty. This is the reason the sheet reads
    /// computedPolicy, and it is asserted rather than assumed.
    #[test]
    fn inherited_policy_comes_from_computed_policy_not_the_spec() {
        let snap = captured_snapshot();
        let pg = snap
            .hosts
            .iter()
            .flat_map(|h| h.array_prop("config.network.portgroup"))
            .next()
            .expect("the captured port group");
        assert!(
            pg.child("spec").and_then(|s| s.child("policy")).and_then(|p| p.child("nicTeaming")).is_none(),
            "the spec should carry no teaming -- that is the point of this test"
        );
        let rows = cells(rows(&snap).expect("named host"));
        assert!(
            matches!(at(&rows[0], "Policy"), Cell::Text(ref s) if s == "loadbalance_srcid"),
            "inherited teaming must still be reported, got {:?}",
            at(&rows[0], "Policy")
        );
    }

    /// A distributed port group is the `dvPort` sheet, never a vPort row.
    #[test]
    fn distributed_port_groups_are_not_rows() {
        let snap = captured_snapshot();
        let standard: usize =
            snap.hosts.iter().map(|h| h.array_prop("config.network.portgroup").len()).sum();
        assert_eq!(cells(rows(&snap).expect("named host")).len(), standard);
    }
}
