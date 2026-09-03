//! vPort — one row per **standard** port group on a host.
//!
//! Reads `config.network.portgroup` off the shared host snapshot.
//!
//! # Zero rows in the reference lab
//!
//! Like vSwitch, this is empty there: the hosts have no standard switch, so
//! they have no standard port group either (`config.network.portgroup` returns
//! an empty array on all three). The path is verified as returned-but-empty;
//! the element shape follows the vim25 schema rather than an observed
//! `HostPortGroup`. Distributed port groups are the `dvPort` sheet.

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
            let policy = spec.and_then(|s| s.child("policy"));
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
    use crate::data::snapshot::InventorySnapshot;
    use crate::vcenter::soap::ManagedObject;
    use crate::vcenter::xml;

    #[test]
    fn the_captured_hosts_have_no_standard_port_group() {
        let snap = captured_snapshot();
        for h in &snap.hosts {
            assert!(h.array_prop("config.network.portgroup").is_empty());
        }
        assert!(cells(rows(&snap).expect("named host")).is_empty());
    }

    /// Synthetic, and marked as such: the lab has no standard port group, so
    /// this asserts the documented schema shape rather than a capture.
    #[test]
    fn a_standard_port_group_becomes_a_row_synthetic_shape() {
        let fragment = r#"<objects><obj type="HostSystem">host-1</obj>
          <propSet><name>name</name><val>esx1</val></propSet>
          <propSet><name>config.network.portgroup</name><val>
            <HostPortGroup>
              <spec><name>Management Network</name><vswitchName>vSwitch0</vswitchName>
                <vlanId>0</vlanId>
                <policy>
                  <security><allowPromiscuous>false</allowPromiscuous>
                    <macChanges>false</macChanges><forgedTransmits>false</forgedTransmits></security>
                  <nicTeaming><policy>loadbalance_srcid</policy></nicTeaming>
                  <shapingPolicy><enabled>false</enabled></shapingPolicy>
                </policy>
              </spec>
            </HostPortGroup>
          </val></propSet></objects>"#;
        let host = ManagedObject::from_element(&xml::parse(fragment).expect("parses"));
        let snap = InventorySnapshot::from_parts(Vec::new(), vec![host]);
        let rows = cells(rows(&snap).expect("named host"));
        assert_eq!(rows.len(), 1);
        let at = |l: &str| rows[0][col(&columns(), l)].clone();
        assert!(matches!(at("Port Group"), Cell::Text(ref s) if s == "Management Network"));
        assert!(matches!(at("Switch"), Cell::Text(ref s) if s == "vSwitch0"));
        assert!(matches!(at("VLAN"), Cell::Number(n) if n == 0.0));
    }
}
