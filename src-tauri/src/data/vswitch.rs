//! vSwitch — one row per **standard** virtual switch.
//!
//! Reads `config.network.vswitch` off the shared host snapshot.
//!
//! The lab runs entirely on a distributed switch and had **no standard vSwitch
//! at all**, so this sheet parsed nothing. One was created on a single host
//! purely so it has something real to read (`sttools-vSwitch`, isolated with no
//! uplinks — see `docs/LAB-ENVIRONMENT.md`), and the fields below are what a
//! real `HostVirtualSwitch` returned.
//!
//! Note `numPorts` is the *elastic* port count ESXi actually allocated (9216),
//! not the 128 that was requested; the request survives under `spec/numPorts`.
//! RVTools shows the allocated figure, which is what this reads.
//!
//! Distributed switches are a different sheet (`dvSwitch`); a host's view of one
//! is a `proxySwitch`, deliberately not counted here, because RVTools' vSwitch
//! is about standard switches and merging the two would double-count.

use super::hostnet::HOST_NET_PROPS;
use super::snapshot::{InventorySnapshot, RowSource, SheetSpec};
use super::{Cell, Column, Table};
use crate::vcenter::VCenterConnection;

pub const HOST_PROPS: &[&str] = HOST_NET_PROPS;

pub fn columns() -> Vec<Column> {
    vec![
        Column::text("Host"),
        Column::text("Switch"),
        Column::number("# Ports"),
        Column::number("Free Ports"),
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
        Column::number("MTU"),
    ]
}

pub fn rows(snap: &InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let mut rows = Vec::new();

    for host in &snap.hosts {
        let Some(name) = host.str_prop("name") else {
            return Err(format!("HostSystem {} returned no name property", host.moref));
        };

        for sw in host.array_prop("config.network.vswitch") {
            let spec = sw.child("spec");
            let policy = spec.and_then(|s| s.child("policy"));
            let num = |p: &str| {
                sw.text_at(p).and_then(|v| v.parse::<f64>().ok())
            };

            rows.push((
                host.moref.clone(),
                vec![
                    Cell::Text(name.clone()),
                    Cell::opt_text(sw.text_at("name").filter(|s| !s.is_empty())),
                    Cell::opt_num(num("numPorts")),
                    Cell::opt_num(num("numPortsAvailable")),
                    // A standard switch states its policy directly, without the
                    // inherited/value envelope a distributed switch uses.
                    Cell::opt_bool(
                        policy.and_then(|p| p.text_at("security/allowPromiscuous"))
                            .map(|v| v == "true"),
                    ),
                    Cell::opt_bool(
                        policy.and_then(|p| p.text_at("security/macChanges")).map(|v| v == "true"),
                    ),
                    Cell::opt_bool(
                        policy
                            .and_then(|p| p.text_at("security/forgedTransmits"))
                            .map(|v| v == "true"),
                    ),
                    Cell::opt_bool(
                        policy.and_then(|p| p.text_at("shapingPolicy/enabled")).map(|v| v == "true"),
                    ),
                    Cell::opt_num(
                        policy
                            .and_then(|p| p.text_at("shapingPolicy/averageBandwidth"))
                            .and_then(|v| v.parse::<f64>().ok()),
                    ),
                    Cell::opt_num(
                        policy
                            .and_then(|p| p.text_at("shapingPolicy/peakBandwidth"))
                            .and_then(|v| v.parse::<f64>().ok()),
                    ),
                    Cell::opt_num(
                        policy
                            .and_then(|p| p.text_at("shapingPolicy/burstSize"))
                            .and_then(|v| v.parse::<f64>().ok()),
                    ),
                    Cell::opt_text(policy.and_then(|p| p.text_at("nicTeaming/policy"))),
                    Cell::opt_bool(
                        policy
                            .and_then(|p| p.text_at("nicTeaming/reversePolicy"))
                            .map(|v| v == "true"),
                    ),
                    Cell::opt_bool(
                        policy
                            .and_then(|p| p.text_at("nicTeaming/notifySwitches"))
                            .map(|v| v == "true"),
                    ),
                    Cell::opt_bool(
                        policy
                            .and_then(|p| p.text_at("nicTeaming/rollingOrder"))
                            .map(|v| v == "true"),
                    ),
                    Cell::opt_num(num("mtu")),
                ],
            ));
        }
    }

    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vSwitch",
    columns,
    vm_props: &[],
    host_props: &[HOST_PROPS],
    dvs_props: &[],
    dvpg_props: &[],
    cluster_props: &[],
    datastore_props: &[],
    rp_props: &[],
    wants_licenses: false,
    wants_about: false,
    source: RowSource::Host,
    rows,
};

pub async fn fetch_vswitch_all(
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

    /// One standard switch exists, on the captured host, and it is the only one
    /// in the lab.
    #[test]
    fn one_row_per_standard_switch() {
        let rows = cells(rows(&captured_snapshot()).expect("named host"));
        assert_eq!(rows.len(), 1);
        assert!(matches!(at(&rows[0], "Switch"), Cell::Text(ref s) if s == "sttools-vSwitch"));
    }

    /// `numPorts` is the elastic count ESXi allocated, not the 128 requested;
    /// the request survives separately under `spec/numPorts`. RVTools shows the
    /// allocated figure.
    #[test]
    fn ports_are_the_allocated_count_not_the_requested_one() {
        let rows = cells(rows(&captured_snapshot()).expect("named host"));
        let ports = at(&rows[0], "# Ports");
        assert!(
            matches!(ports, Cell::Number(n) if n > 128.0),
            "expected the elastic allocation, got {ports:?}"
        );
        assert!(matches!(at(&rows[0], "Free Ports"), Cell::Number(_)));
        assert!(matches!(at(&rows[0], "MTU"), Cell::Number(n) if n == 1500.0));
    }

    /// A standard switch states its policy directly under `spec/policy`, with
    /// none of the inherited/value wrapping a distributed switch uses. ESXi
    /// fills in the defaults, so all of it comes back even though only a bare
    /// spec was submitted.
    #[test]
    fn policy_is_read_without_an_inheritance_envelope() {
        let rows = cells(rows(&captured_snapshot()).expect("named host"));
        let r = &rows[0];
        assert!(matches!(at(r, "Promiscuous Mode"), Cell::Bool(false)));
        assert!(matches!(at(r, "Mac Changes"), Cell::Bool(false)));
        assert!(matches!(at(r, "Policy"), Cell::Text(ref s) if s == "loadbalance_srcid"));
        assert!(matches!(at(r, "Notify Switch"), Cell::Bool(true)));
        assert!(matches!(at(r, "Traffic Shaping"), Cell::Bool(false)));
    }

    /// A host's view of a *distributed* switch is a `proxySwitch`. It must not
    /// become a vSwitch row: RVTools counts standard switches here, and counting
    /// both would double-report the same networking.
    #[test]
    fn a_proxy_switch_is_not_a_standard_switch() {
        let snap = captured_snapshot();
        let proxies: usize =
            snap.hosts.iter().map(|h| h.array_prop("config.network.proxySwitch").len()).sum();
        assert!(proxies > 0, "the captured host does have a proxy switch");
        // Still exactly one row: the standard switch, not the proxy.
        assert_eq!(cells(rows(&snap).expect("named host")).len(), 1);
    }
}
