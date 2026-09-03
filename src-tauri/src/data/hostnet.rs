//! Shared plumbing for the host networking sheets.
//!
//! vHBA, vNIC, vSwitch, vPort and vSC_VMK all read arrays off one `HostSystem`
//! fetch, so they declare property sets from here rather than restating them,
//! and share the two lookups that turn a physical NIC into the switch and
//! uplink port it serves.

use crate::vcenter::soap::ManagedObject;
use crate::vcenter::xml::Element;
use std::collections::HashMap;

/// `HostSystem` properties the networking sheets read.
pub const HOST_NET_PROPS: &[&str] = &[
    "name",
    "config.network.pnic",
    "config.network.vswitch",
    "config.network.portgroup",
    "config.network.vnic",
    "config.network.proxySwitch",
];

/// `HostSystem` properties vHBA reads.
pub const HOST_HBA_PROPS: &[&str] = &["name", "config.storageDevice.hostBusAdapter"];

/// Where a physical NIC is attached, as the host sees it.
#[derive(Debug, Clone, Default)]
pub struct PnicAttachment {
    /// The distributed switch's name, from the host's proxy switch.
    pub switch: Option<String>,
    /// The uplink port name (`uplink1`), not the numeric key it is stored under.
    pub uplink_port: Option<String>,
}

/// `vmnicN` → the switch and uplink port it backs.
///
/// A host's view of a distributed switch is a `proxySwitch`, whose
/// `spec/backing/pnicSpec` lists which physical NIC serves which uplink. The
/// uplink is given as a numeric `uplinkPortKey`, and the readable name lives in
/// the proxy switch's own `uplinkPort` key/value pairs — so resolving it needs
/// both halves. Verified against the lab, where two of six NICs per host are
/// attached and the rest are unused.
pub fn pnic_attachments(host: &ManagedObject) -> HashMap<String, PnicAttachment> {
    let mut out = HashMap::new();
    for proxy in host.array_prop("config.network.proxySwitch") {
        let switch = proxy.text_at("dvsName").filter(|s| !s.is_empty());

        // uplink port key -> readable name
        let mut port_names: HashMap<String, String> = HashMap::new();
        for up in proxy.children_named("uplinkPort") {
            if let (Some(k), Some(v)) = (up.text_at("key"), up.text_at("value")) {
                port_names.insert(k, v);
            }
        }

        let Some(backing) = proxy.child("spec").and_then(|s| s.child("backing")) else {
            continue;
        };
        for spec in backing.children_named("pnicSpec") {
            let Some(device) = spec.text_at("pnicDevice").filter(|s| !s.is_empty()) else {
                continue;
            };
            let uplink_port = spec
                .text_at("uplinkPortKey")
                .and_then(|k| port_names.get(&k).cloned());
            out.insert(
                device,
                PnicAttachment { switch: switch.clone(), uplink_port },
            );
        }
    }
    out
}

/// Read a `<field><value>x</value></field>` pair.
///
/// Distributed-switch settings are wrapped in an inheritance envelope: each
/// setting is an object carrying `inherited` plus the effective `value`. The
/// value is what RVTools shows; `inherited` says only where it came from.
pub fn policy_value(root: Option<&Element>, path: &str) -> Option<String> {
    let node = walk(root?, path)?;
    node.text_at("value").filter(|s| !s.is_empty())
}

/// Follow a slash-separated path of child element names.
pub fn walk<'a>(root: &'a Element, path: &str) -> Option<&'a Element> {
    let mut node = root;
    for seg in path.split('/') {
        node = node.child(seg)?;
    }
    Some(node)
}

/// A policy value that is a boolean.
pub fn policy_bool(root: Option<&Element>, path: &str) -> Option<bool> {
    match policy_value(root, path)?.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// A policy value that is a number.
pub fn policy_num(root: Option<&Element>, path: &str) -> Option<f64> {
    policy_value(root, path)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcenter::xml;

    /// The uplink is stored as a numeric key on the pnic and named elsewhere on
    /// the proxy switch; resolving it needs both halves, which is the whole
    /// reason this helper exists.
    #[test]
    fn a_pnic_resolves_its_switch_and_uplink_port_name() {
        let fragment = r#"<objects><obj type="HostSystem">host-1</obj>
          <propSet><name>config.network.proxySwitch</name><val>
            <HostProxySwitch>
              <dvsName>vds01</dvsName>
              <uplinkPort><key>16</key><value>uplink2</value></uplinkPort>
              <uplinkPort><key>17</key><value>uplink1</value></uplinkPort>
              <spec><backing>
                <pnicSpec><pnicDevice>vmnic4</pnicDevice><uplinkPortKey>17</uplinkPortKey></pnicSpec>
                <pnicSpec><pnicDevice>vmnic5</pnicDevice><uplinkPortKey>16</uplinkPortKey></pnicSpec>
              </backing></spec>
            </HostProxySwitch>
          </val></propSet></objects>"#;
        let host = ManagedObject::from_element(&xml::parse(fragment).expect("parses"));
        let map = pnic_attachments(&host);
        assert_eq!(map["vmnic4"].switch.as_deref(), Some("vds01"));
        assert_eq!(map["vmnic4"].uplink_port.as_deref(), Some("uplink1"));
        assert_eq!(map["vmnic5"].uplink_port.as_deref(), Some("uplink2"));
        // An unattached NIC simply is not in the map.
        assert!(!map.contains_key("vmnic0"));
    }

    /// Every distributed-switch setting is wrapped in an inheritance envelope.
    /// Reading the field directly, rather than its `value` child, yields empty.
    #[test]
    fn a_policy_value_is_read_from_inside_its_envelope() {
        let el = xml::parse(
            r#"<defaultPortConfig>
                 <securityPolicy>
                   <inherited>false</inherited>
                   <allowPromiscuous><inherited>false</inherited><value>true</value></allowPromiscuous>
                 </securityPolicy>
               </defaultPortConfig>"#,
        )
        .expect("parses");
        assert_eq!(
            policy_bool(Some(&el), "securityPolicy/allowPromiscuous"),
            Some(true)
        );
        assert_eq!(policy_value(Some(&el), "securityPolicy/nope"), None);
    }
}
