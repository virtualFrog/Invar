//! vSwitch — one row per **standard** virtual switch.
//!
//! Reads `config.network.vswitch` off the shared host snapshot.
//!
//! # Zero rows in the reference lab
//!
//! That lab's hosts run entirely on a distributed switch and have **no standard
//! vSwitch at all** (`config.network.vswitch` returns an empty array on all
//! three hosts), so this sheet produces nothing there and its parsing has not
//! run against a real response. The property path itself is verified — it is
//! returned, just empty — but the element shape below follows the vim25 schema
//! rather than an observed `HostVirtualSwitch`.
//!
//! Distributed switches are a different sheet (`dvSwitch`); a host's view of one
//! is a `proxySwitch`, deliberately not counted here, because RVTools' vSwitch
//! is about standard switches and merging the two would double-count.

use super::hostnet::{policy_bool, policy_num, policy_value, HOST_NET_PROPS};
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

    // Silence unused-import warnings when the policy helpers are only needed by
    // the distributed sheets.
    let _ = (policy_bool, policy_num, policy_value);
    Ok(rows)
}

pub const SPEC: SheetSpec = SheetSpec {
    name: "vSwitch",
    columns,
    vm_props: &[],
    host_props: &[HOST_PROPS],
    dvs_props: &[],
    dvpg_props: &[],
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
    use crate::data::snapshot::InventorySnapshot;
    use crate::vcenter::soap::ManagedObject;
    use crate::vcenter::xml;

    /// The captured hosts run entirely on a distributed switch, so this is
    /// genuinely empty. Asserted rather than assumed, so a future capture that
    /// does contain one is noticed.
    #[test]
    fn the_captured_hosts_have_no_standard_switch() {
        let snap = captured_snapshot();
        for h in &snap.hosts {
            assert!(h.array_prop("config.network.vswitch").is_empty());
        }
        assert!(cells(rows(&snap).expect("named host")).is_empty());
    }

    /// A host's view of a *distributed* switch is a `proxySwitch`. It must not
    /// become a vSwitch row: RVTools counts standard switches here, and
    /// counting both would double-report the same networking.
    #[test]
    fn a_proxy_switch_is_not_a_standard_switch() {
        let snap = captured_snapshot();
        let has_proxy = snap
            .hosts
            .iter()
            .any(|h| !h.array_prop("config.network.proxySwitch").is_empty());
        assert!(has_proxy, "the captured host does have a proxy switch");
        assert!(cells(rows(&snap).expect("named host")).is_empty());
    }

    /// Synthetic, and marked as such: no standard vSwitch exists in the lab, so
    /// this asserts the shape the vim25 schema documents rather than one that
    /// was captured. It is here so the parsing is exercised at all.
    #[test]
    fn a_standard_switch_becomes_a_row_synthetic_shape() {
        let fragment = r#"<objects><obj type="HostSystem">host-1</obj>
          <propSet><name>name</name><val>esx1</val></propSet>
          <propSet><name>config.network.vswitch</name><val>
            <HostVirtualSwitch>
              <name>vSwitch0</name><numPorts>128</numPorts>
              <numPortsAvailable>120</numPortsAvailable><mtu>1500</mtu>
              <spec><policy>
                <security><allowPromiscuous>false</allowPromiscuous>
                  <macChanges>true</macChanges><forgedTransmits>true</forgedTransmits></security>
                <nicTeaming><policy>loadbalance_srcid</policy>
                  <reversePolicy>true</reversePolicy><notifySwitches>true</notifySwitches>
                  <rollingOrder>false</rollingOrder></nicTeaming>
                <shapingPolicy><enabled>false</enabled></shapingPolicy>
              </policy></spec>
            </HostVirtualSwitch>
          </val></propSet></objects>"#;
        let host = ManagedObject::from_element(&xml::parse(fragment).expect("parses"));
        let snap = InventorySnapshot::from_parts(Vec::new(), vec![host]);
        let rows = cells(rows(&snap).expect("named host"));
        assert_eq!(rows.len(), 1);
        let at = |l: &str| rows[0][col(&columns(), l)].clone();
        assert!(matches!(at("Switch"), Cell::Text(ref s) if s == "vSwitch0"));
        assert!(matches!(at("# Ports"), Cell::Number(n) if n == 128.0));
        assert!(matches!(at("Promiscuous Mode"), Cell::Bool(false)));
        assert!(matches!(at("Policy"), Cell::Text(ref s) if s == "loadbalance_srcid"));
    }
}
