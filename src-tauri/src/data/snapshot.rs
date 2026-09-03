//! One inventory fetch, shared by every sheet derived from it.
//!
//! Sheets used to walk the inventory themselves: four of them each ran their
//! own `retrieve("VirtualMachine", ...)`, and `common` added two more walks, so
//! five sheets cost ten full passes, measured live rather than estimated. Ten
//! of RVTools' 27 sheets are
//! VM-derived, so that shape does not survive the parity work in
//! `docs/PARITY-PLAN.md`.
//!
//! Instead a snapshot is fetched once per vCenter, with the union of the
//! requested sheets' property sets, and each sheet is a pure function over it.
//! The second payoff matters as much as the round trips: a sheet that does no
//! I/O is testable against captured XML with no live vCenter.

use super::{Cell, Column, Table};
use crate::vcenter::soap::ManagedObject;
use crate::vcenter::{Session, SessionCache, VCenterConnection};
use std::collections::HashMap;

/// What a sheet's rows are *about*, which decides the location columns it gets.
///
/// RVTools is not uniform here: VM sheets carry Datacenter, Cluster and Folder;
/// vHost carries Datacenter and Cluster but no Folder, because a host's folder
/// is the datacenter's `host` folder and RVTools does not show it; vHealth
/// carries none, being three columns wide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowSource {
    Vm,
    Host,
    /// Rows that describe no single inventory object, or a sheet RVTools gives
    /// no location columns.
    None,
}

/// One node of the inventory tree: its name and the parent it hangs off.
struct PathNode {
    name: String,
    /// `(moref, declared managed-object type)`. `None` at the root folder.
    parent: Option<(String, String)>,
    /// For a VM, the `HostSystem` it runs on. A VM reaches its cluster through
    /// its host, not through its folder: folders and compute live in separate
    /// branches of the inventory tree.
    host: Option<String>,
}

/// Resolves an object's Datacenter, Cluster and Folder by walking `parent`.
///
/// Built from one extra inventory walk over Folder + Datacenter +
/// ComputeResource (a `ComputeResource` view also returns its
/// `ClusterComputeResource` subclass, verified against the lab), plus the
/// `parent` property already carried on each VM and host. Three columns on
/// roughly twenty eventual sheets for one round trip.
#[derive(Default)]
pub struct PathIndex {
    nodes: HashMap<String, PathNode>,
}

impl PathIndex {
    /// Walk up from `moref` until a `Datacenter` is reached.
    ///
    /// The chain is bounded by `nodes.len()` rather than trusted to terminate:
    /// a cycle in inventory data would otherwise hang the export.
    pub fn datacenter_of(&self, moref: &str) -> Option<String> {
        let mut at = moref.to_string();
        for _ in 0..=self.nodes.len() {
            let node = self.nodes.get(&at)?;
            let (parent, kind) = node.parent.as_ref()?;
            if kind == "Datacenter" {
                return self.nodes.get(parent).map(|n| n.name.clone());
            }
            at = parent.clone();
        }
        None
    }

    /// The name of the folder a VM sits in directly.
    pub fn folder_of(&self, vm_moref: &str) -> Option<String> {
        let (parent, kind) = self.nodes.get(vm_moref)?.parent.as_ref()?;
        (kind == "Folder").then(|| self.nodes.get(parent).map(|n| n.name.clone()))?
    }

    /// A host's cluster, or `None` for a standalone host.
    ///
    /// A host outside a cluster still has a `ComputeResource` parent, but that
    /// is a container vSphere invents rather than a cluster anyone named, so
    /// reporting it would invent a cluster that does not exist.
    pub fn cluster_of_host(&self, host_moref: &str) -> Option<String> {
        let (parent, kind) = self.nodes.get(host_moref)?.parent.as_ref()?;
        (kind == "ClusterComputeResource")
            .then(|| self.nodes.get(parent).map(|n| n.name.clone()))?
    }

    /// The name of any indexed object, by moref.
    ///
    /// vNetwork uses this to resolve a NIC's portgroup reference: a
    /// distributed-port backing names its portgroup by moref
    /// (`dvportgroup-9335`), never by name.
    pub fn name_of(&self, moref: &str) -> Option<String> {
        self.nodes
            .get(moref)
            .map(|n| n.name.clone())
            .filter(|n| !n.is_empty())
    }

    /// A VM's cluster, reached through the host it runs on.
    pub fn cluster_of_vm(&self, vm_moref: &str) -> Option<String> {
        let host = self.nodes.get(vm_moref)?.host.as_ref()?;
        self.cluster_of_host(host)
    }
}

/// What one vCenter returned for a fetch.
pub struct InventorySnapshot {
    /// The `VI SDK Server` value for rows built from this snapshot.
    pub server: String,
    pub vms: Vec<ManagedObject>,
    pub hosts: Vec<ManagedObject>,
    /// `HostSystem` moref → host name, for resolving `runtime.host`. Derived
    /// from `hosts`, so it costs no extra round trip.
    pub host_names: HashMap<String, String>,
    /// `DistributedVirtualSwitch` objects, for dvSwitch.
    pub dvswitches: Vec<ManagedObject>,
    /// `DistributedVirtualPortgroup` objects, for dvPort.
    pub dvportgroups: Vec<ManagedObject>,
    /// `ClusterComputeResource` objects, for vCluster.
    pub clusters: Vec<ManagedObject>,
    /// `Datastore` objects, for vDatastore.
    pub datastores: Vec<ManagedObject>,
    /// `ResourcePool` objects, for vRP.
    pub resource_pools: Vec<ManagedObject>,
    /// The `LicenseManager` singleton, for vLicense. Not reachable through a
    /// ContainerView, so it is fetched by its own moref.
    pub license_manager: Option<ManagedObject>,
    /// `ServiceContent.about`, for vSource.
    pub about: Option<crate::vcenter::xml::Element>,
    /// Datacenter / Cluster / Folder lookups.
    pub paths: PathIndex,
}

impl InventorySnapshot {
    /// Retrieve only the object types the caller asked for. An empty property
    /// set means that type is not queried at all.
    pub async fn fetch(
        session: &Session,
        server: &str,
        vm_props: &[&'static str],
        host_props: &[&'static str],
        dvs_props: &[&'static str],
        dvpg_props: &[&'static str],
        cluster_props: &[&'static str],
        datastore_props: &[&'static str],
        rp_props: &[&'static str],
        want_licenses: bool,
        want_about: bool,
    ) -> Result<Self, String> {
        // Every VM-derived sheet resolves `runtime.host` to a host name, so a
        // VM fetch always implies at least the hosts' names. This is the walk
        // `common::host_names` used to do on its own.
        // `parent` is what the inventory path index walks, and the location
        // columns are appended to every sheet, so it is never optional.
        let host_props: Vec<&'static str> = if vm_props.is_empty() {
            union(&[host_props, &["parent"]])
        } else {
            union(&[host_props, &["name", "parent"]])
        };
        let vm_props: Vec<&'static str> = if vm_props.is_empty() {
            Vec::new()
        } else {
            union(&[vm_props, &["parent", "runtime.host"]])
        };

        let hosts = if host_props.is_empty() {
            Vec::new()
        } else {
            session.soap.retrieve("HostSystem", &host_props).await?
        };

        let vms = if vm_props.is_empty() {
            Vec::new()
        } else {
            session.soap.retrieve("VirtualMachine", &vm_props).await?
        };

        // One walk covers every container type. A ComputeResource view also
        // returns ClusterComputeResource, and a Network view also returns
        // DistributedVirtualPortgroup — both subclasses, so neither needs a
        // query of its own. `Network` is here so vNetwork can turn a NIC's
        // portgroup moref into the name RVTools shows. All four types carry
        // `name` and `parent`, which is why one shared pathSet works.
        let containers = session
            .soap
            .retrieve_types(
                &[
                    "Folder",
                    "Datacenter",
                    "ComputeResource",
                    "Network",
                    "DistributedVirtualSwitch",
                    // vInfo names a VM's resource pool, which arrives as a moref.
                    "ResourcePool",
                ],
                &["name", "parent"],
            )
            .await?;

        // Distributed switching needs its own object types. A DVS and a
        // portgroup carry different properties, so unlike the container walk
        // these cannot share one propSet and are fetched separately -- and only
        // when a sheet actually asks for them.
        let dvswitches = if dvs_props.is_empty() {
            Vec::new()
        } else {
            session.soap.retrieve("DistributedVirtualSwitch", dvs_props).await?
        };
        let dvportgroups = if dvpg_props.is_empty() {
            Vec::new()
        } else {
            session.soap.retrieve("DistributedVirtualPortgroup", dvpg_props).await?
        };

        let clusters = if cluster_props.is_empty() {
            Vec::new()
        } else {
            session.soap.retrieve("ClusterComputeResource", cluster_props).await?
        };
        let datastores = if datastore_props.is_empty() {
            Vec::new()
        } else {
            session.soap.retrieve("Datastore", datastore_props).await?
        };
        let resource_pools = if rp_props.is_empty() {
            Vec::new()
        } else {
            session.soap.retrieve("ResourcePool", rp_props).await?
        };

        // Neither of these lives in a container, so both are direct calls.
        let about = if want_about {
            session.soap.service_content().await?.child("about").cloned()
        } else {
            None
        };
        let license_manager = if want_licenses {
            session
                .soap
                .retrieve_moref("LicenseManager", "LicenseManager", &["licenses"])
                .await?
        } else {
            None
        };

        let host_names = hosts
            .iter()
            .filter_map(|h| h.str_prop("name").map(|n| (h.moref.clone(), n)))
            .collect();
        let paths = PathIndex::build(&containers, &vms, &hosts);

        Ok(Self {
            server: server.to_string(),
            vms,
            hosts,
            host_names,
            dvswitches,
            dvportgroups,
            clusters,
            datastores,
            resource_pools,
            license_manager,
            about,
            paths,
        })
    }

    /// A snapshot assembled by hand, for tests that have captured XML but no
    /// vCenter to fetch from.
    pub fn from_parts(vms: Vec<ManagedObject>, hosts: Vec<ManagedObject>) -> Self {
        Self::from_parts_with_containers(vms, hosts, Vec::new())
    }

    /// As `from_parts`, plus the Folder / Datacenter / ComputeResource objects
    /// the path index is built from.
    pub fn from_parts_with_containers(
        vms: Vec<ManagedObject>,
        hosts: Vec<ManagedObject>,
        containers: Vec<ManagedObject>,
    ) -> Self {
        let host_names = hosts
            .iter()
            .filter_map(|h| h.str_prop("name").map(|n| (h.moref.clone(), n)))
            .collect();
        let paths = PathIndex::build(&containers, &vms, &hosts);
        Self {
            server: "test".into(),
            vms,
            hosts,
            host_names,
            dvswitches: Vec::new(),
            dvportgroups: Vec::new(),
            clusters: Vec::new(),
            datastores: Vec::new(),
            resource_pools: Vec::new(),
            license_manager: None,
            about: None,
            paths,
        }
    }

    pub fn with_clusters(mut self, clusters: Vec<ManagedObject>) -> Self {
        self.clusters = clusters;
        self
    }

    pub fn with_datastores(mut self, datastores: Vec<ManagedObject>) -> Self {
        self.datastores = datastores;
        self
    }

    pub fn with_resource_pools(mut self, pools: Vec<ManagedObject>) -> Self {
        self.resource_pools = pools;
        self
    }

    pub fn with_license_manager(mut self, lm: Option<ManagedObject>) -> Self {
        self.license_manager = lm;
        self
    }

    pub fn with_about(mut self, about: crate::vcenter::xml::Element) -> Self {
        self.about = Some(about);
        self
    }

    /// As `from_parts_with_containers`, plus distributed-switching objects.
    pub fn with_distributed(
        mut self,
        dvswitches: Vec<ManagedObject>,
        dvportgroups: Vec<ManagedObject>,
    ) -> Self {
        self.dvswitches = dvswitches;
        self.dvportgroups = dvportgroups;
        self
    }
}

impl PathIndex {
    /// Containers supply the tree; VMs and hosts hang off it via their own
    /// `parent`, which their sheets already fetch.
    fn build(
        containers: &[ManagedObject],
        vms: &[ManagedObject],
        hosts: &[ManagedObject],
    ) -> Self {
        let mut nodes = HashMap::new();
        for c in containers {
            nodes.insert(
                c.moref.clone(),
                PathNode {
                    name: c.str_prop("name").unwrap_or_default(),
                    parent: c.moref_prop("parent"),
                    host: None,
                },
            );
        }
        for h in hosts {
            nodes.insert(
                h.moref.clone(),
                PathNode {
                    name: h.str_prop("name").unwrap_or_default(),
                    parent: h.moref_prop("parent"),
                    host: None,
                },
            );
        }
        for vm in vms {
            nodes.insert(
                vm.moref.clone(),
                PathNode {
                    name: vm.str_prop("name").unwrap_or_default(),
                    parent: vm.moref_prop("parent"),
                    host: vm.str_prop("runtime.host"),
                },
            );
        }
        Self { nodes }
    }
}

/// Merge property sets, preserving first-seen order and dropping duplicates.
///
/// Order is kept because vCenter echoes `propSet` back in request order, and a
/// stable request makes captured XML fixtures comparable between runs.
pub fn union(sets: &[&[&'static str]]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for set in sets {
        for prop in *set {
            if !out.contains(prop) {
                out.push(prop);
            }
        }
    }
    out
}

/// Everything the app needs to know about one sheet.
///
/// Adding a sheet is a new module plus one entry in `data::SHEETS`. The UI
/// needs no change: it renders whatever `list_sheets` and `fetch_sheet` return.
pub struct SheetSpec {
    /// RVTools' sheet name, e.g. `vInfo`.
    pub name: &'static str,
    pub columns: fn() -> Vec<Column>,
    /// `VirtualMachine` property sets this sheet reads, unioned at fetch time.
    /// Sets rather than one flat list so a sheet composes shared groups like
    /// `common::VM_CONTEXT_PROPS` instead of copying them. Empty reads nothing.
    pub vm_props: &'static [&'static [&'static str]],
    /// `HostSystem` property sets this sheet reads. Empty reads nothing.
    pub host_props: &'static [&'static [&'static str]],
    /// `DistributedVirtualSwitch` property sets. Empty reads nothing.
    pub dvs_props: &'static [&'static [&'static str]],
    /// `DistributedVirtualPortgroup` property sets. Empty reads nothing.
    pub dvpg_props: &'static [&'static [&'static str]],
    /// `ClusterComputeResource` property sets. Empty reads nothing.
    pub cluster_props: &'static [&'static [&'static str]],
    /// `Datastore` property sets. Empty reads nothing.
    pub datastore_props: &'static [&'static [&'static str]],
    /// `ResourcePool` property sets. Empty reads nothing.
    pub rp_props: &'static [&'static [&'static str]],
    /// Whether this sheet needs the `LicenseManager` singleton.
    pub wants_licenses: bool,
    /// Whether this sheet needs `ServiceContent.about`.
    pub wants_about: bool,
    /// What each row describes, which decides its location columns.
    pub source: RowSource,
    /// Pure by design: all I/O happened when the snapshot was built.
    ///
    /// Each row is paired with the moref of the object it describes, so
    /// Datacenter / Cluster / Folder are resolved once in `Table::extend_from`
    /// rather than in every sheet.
    pub rows: fn(&InventorySnapshot) -> Result<Vec<(String, Vec<Cell>)>, String>,
}

/// Build every sheet in `specs` from one snapshot per vCenter.
///
/// Never fails as a whole. A vCenter that cannot be reached contributes a
/// warning to every table and no rows, so the healthy servers' data still
/// arrives: for an inventory tool, a short list that looks complete is the
/// worst outcome.
pub async fn fetch_tables(
    specs: &[&SheetSpec],
    conns: &[VCenterConnection],
    cache: &SessionCache,
) -> Vec<Table> {
    let vm_sets: Vec<&[&'static str]> =
        specs.iter().flat_map(|s| s.vm_props.iter().copied()).collect();
    let host_sets: Vec<&[&'static str]> =
        specs.iter().flat_map(|s| s.host_props.iter().copied()).collect();
    let dvs_sets: Vec<&[&'static str]> =
        specs.iter().flat_map(|s| s.dvs_props.iter().copied()).collect();
    let dvpg_sets: Vec<&[&'static str]> =
        specs.iter().flat_map(|s| s.dvpg_props.iter().copied()).collect();
    let cluster_sets: Vec<&[&'static str]> =
        specs.iter().flat_map(|s| s.cluster_props.iter().copied()).collect();
    let ds_sets: Vec<&[&'static str]> =
        specs.iter().flat_map(|s| s.datastore_props.iter().copied()).collect();
    let rp_sets: Vec<&[&'static str]> =
        specs.iter().flat_map(|s| s.rp_props.iter().copied()).collect();
    let vm_props = union(&vm_sets);
    let host_props = union(&host_sets);
    let dvs_props = union(&dvs_sets);
    let dvpg_props = union(&dvpg_sets);
    let cluster_props = union(&cluster_sets);
    let datastore_props = union(&ds_sets);
    let rp_props = union(&rp_sets);
    let want_licenses = specs.iter().any(|s| s.wants_licenses);
    let want_about = specs.iter().any(|s| s.wants_about);

    let mut tables: Vec<Table> = specs
        .iter()
        .map(|s| {
            Table::new(s.name, (s.columns)())
                .with_location_columns(s.source)
                .with_source_column()
        })
        .collect();

    for conn in conns {
        let label = conn.label();

        let snapshot = match cache.get(conn).await {
            Ok(session) => {
                InventorySnapshot::fetch(
                    &session,
                    &label,
                    &vm_props,
                    &host_props,
                    &dvs_props,
                    &dvpg_props,
                    &cluster_props,
                    &datastore_props,
                    &rp_props,
                    want_licenses,
                    want_about,
                )
                .await
            }
            Err(e) => Err(e),
        };

        let snapshot = match snapshot {
            Ok(s) => s,
            Err(e) => {
                for table in &mut tables {
                    table.warnings.push(format!("{label}: {e}"));
                }
                continue;
            }
        };

        for (spec, table) in specs.iter().zip(tables.iter_mut()) {
            match (spec.rows)(&snapshot) {
                Ok(rows) => table.extend_from(&label, rows, spec.source, &snapshot.paths),
                Err(e) => table.warnings.push(format!("{label}: {e}")),
            }
        }
    }

    tables
}

/// One sheet, fetching only the properties that sheet reads.
///
/// The interactive path uses this so opening a single tab does not pay for the
/// whole export's property union.
pub async fn fetch_table(
    spec: &SheetSpec,
    conns: &[VCenterConnection],
    cache: &SessionCache,
) -> Table {
    fetch_tables(&[spec], conns, cache)
        .await
        .pop()
        .expect("one spec yields one table")
}

/// Test-only helpers for building a snapshot out of XML fragments.
///
/// This is the fixture harness Phase 0 of `docs/PARITY-PLAN.md` calls for: a
/// sheet is now a pure function, so it can be exercised with no live vCenter.
/// Real captured `RetrievePropertiesEx` responses drop in as the same shape.
#[cfg(test)]
pub mod test_support {
    use super::*;
    use crate::vcenter::xml;

    /// One `<objects>` entry, shaped the way a real response is.
    pub fn object(moref_type: &str, moref: &str, props: &[(&str, &str)]) -> ManagedObject {
        let props: String = props
            .iter()
            .map(|(name, val)| format!("<propSet><name>{name}</name><val>{val}</val></propSet>"))
            .collect();
        let fragment = format!(
            r#"<objects><obj type="{moref_type}">{moref}</obj>{props}</objects>"#
        );
        ManagedObject::from_element(&xml::parse(&fragment).expect("fragment parses"))
    }

    pub fn vm(moref: &str, props: &[(&str, &str)]) -> ManagedObject {
        object("VirtualMachine", moref, props)
    }

    pub fn host(moref: &str, props: &[(&str, &str)]) -> ManagedObject {
        object("HostSystem", moref, props)
    }

    // ---- real captures -------------------------------------------------
    //
    // Each of these is one `<objects>` element lifted verbatim out of a live
    // `RetrievePropertiesEx` response, with identifying values replaced
    // (hostnames, IPs, chassis serials, UUIDs). Element names, `xsi:type`
    // attributes, nesting and ordering are exactly what vCenter returned, which
    // is the whole point: hand-written fragments only prove the parser handles
    // shapes we already imagined. The `xmlns:xsi` declaration rode on the SOAP
    // envelope in the original and is re-declared on the fragment root so each
    // file parses standalone.
    //
    // Captured 2026-09-03 from the lab in `docs/LAB-ENVIRONMENT.md`.

    /// 6 disks across two controllers, with `storageIOAllocation` on each.
    pub const VM_MULTI_DISK: &str = include_str!("fixtures/vm_multi_disk.xml");
    /// One snapshot, and a name containing spaces and parentheses.
    pub const VM_SNAPSHOTS: &str = include_str!("fixtures/vm_snapshots.xml");
    /// A `VirtualCdrom` that is actually connected.
    pub const VM_CONNECTED_CDROM: &str = include_str!("fixtures/vm_connected_cdrom.xml");
    /// `config.template = true`, powered off, `.vmtx` rather than `.vmx`.
    pub const VM_TEMPLATE: &str = include_str!("fixtures/vm_template.xml");
    /// The one VM carrying a `VirtualUSB` device and a *nested* snapshot.
    ///
    /// Built in the lab on purpose: nothing there had either shape, so both
    /// code paths were previously exercised only by hand-written XML. See
    /// `docs/LAB-ENVIRONMENT.md`.
    pub const VM_USB_NESTED_SNAPSHOT: &str =
        include_str!("fixtures/vm_usb_nested_snapshot.xml");
    /// A host with all 40 properties vHost and vHealth read.
    pub const HOST_FULL: &str = include_str!("fixtures/host_full.xml");
    /// The Folder / Datacenter / ClusterComputeResource chain the VM and host
    /// captures hang off: every ancestor up to the datacenter, so the path walk
    /// is exercised over a complete tree rather than a stub.
    pub const CONTAINERS: &str = include_str!("fixtures/containers.xml");

    /// Parse one captured `<objects>` element.
    pub fn captured(xml: &str) -> ManagedObject {
        ManagedObject::from_element(&xml::parse(xml).expect("captured fixture parses"))
    }

    /// The lab's one distributed switch, with its full `config` and `summary`.
    pub const DVSWITCHES: &str = include_str!("fixtures/dvswitches.xml");
    /// Three representative distributed port groups. Three rather than all 60:
    /// the config of one is ~5 KB, and three cover the shapes that differ.
    pub const DVPORTGROUPS: &str = include_str!("fixtures/dvportgroups.xml");

    pub fn captured_dvswitches() -> Vec<ManagedObject> {
        captured_many(DVSWITCHES)
    }

    pub fn captured_dvportgroups() -> Vec<ManagedObject> {
        captured_many(DVPORTGROUPS)
    }

    pub const CLUSTERS: &str = include_str!("fixtures/clusters.xml");
    pub const DATASTORES: &str = include_str!("fixtures/datastores.xml");
    /// Four of the lab's 43 pools: the cluster root plus three namespace pools.
    pub const RESOURCE_POOLS: &str = include_str!("fixtures/resourcepools.xml");
    /// The `LicenseManager` singleton. Its key is masked -- a real licence key
    /// is a credential and does not belong in a public repo.
    pub const LICENSES: &str = include_str!("fixtures/licenses.xml");
    /// `ServiceContent.about`, which is not a managed object at all.
    pub const ABOUT: &str = include_str!("fixtures/about.xml");

    pub fn captured_clusters() -> Vec<ManagedObject> {
        captured_many(CLUSTERS)
    }
    pub fn captured_datastores() -> Vec<ManagedObject> {
        captured_many(DATASTORES)
    }
    pub fn captured_resource_pools() -> Vec<ManagedObject> {
        captured_many(RESOURCE_POOLS)
    }
    pub fn captured_licenses() -> ManagedObject {
        captured_many(LICENSES).into_iter().next().expect("one LicenseManager")
    }
    pub fn captured_about() -> crate::vcenter::xml::Element {
        xml::parse(ABOUT).expect("about fixture parses")
    }

    /// Parse a capture holding several `<objects>` elements under one root.
    pub fn captured_many(xml: &str) -> Vec<ManagedObject> {
        let root = xml::parse(xml).expect("captured fixture parses");
        root.children_named("objects")
            .map(ManagedObject::from_element)
            .collect()
    }

    /// A snapshot assembled from the real captures: four VMs and one host.
    ///
    /// Only `VM_SNAPSHOTS` sits on the captured host (`host-12`); the other
    /// three reference `host-28`, which is deliberately absent, so the
    /// unresolved-moref fallback is exercised by real data rather than by a
    /// contrived moref.
    pub fn captured_snapshot() -> InventorySnapshot {
        InventorySnapshot::from_parts_with_containers(
            vec![
                captured(VM_MULTI_DISK),
                captured(VM_SNAPSHOTS),
                captured(VM_CONNECTED_CDROM),
                captured(VM_TEMPLATE),
                captured(VM_USB_NESTED_SNAPSHOT),
            ],
            vec![captured(HOST_FULL)],
            captured_many(CONTAINERS),
        )
    }

    /// Drop the source moref from a sheet's rows.
    ///
    /// `SheetSpec::rows` pairs every row with the moref of the object it
    /// describes so `Table::extend_from` can resolve Datacenter / Cluster /
    /// Folder in one place. Tests that only care about cell values strip it.
    pub fn cells(rows: Vec<(String, Vec<Cell>)>) -> Vec<Vec<Cell>> {
        rows.into_iter().map(|(_, cells)| cells).collect()
    }

    /// Index of a column by its RVTools label, so tests never hard-code a
    /// position that a new column would silently shift.
    pub fn col(columns: &[Column], label: &str) -> usize {
        columns
            .iter()
            .position(|c| c.label == label)
            .unwrap_or_else(|| panic!("no column labelled {label}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_dedupes_and_keeps_first_seen_order() {
        let merged = union(&[
            &["name", "runtime.host"],
            &["runtime.host", "config.template"],
            &["name"],
        ]);
        assert_eq!(merged, vec!["name", "runtime.host", "config.template"]);
    }

    #[test]
    fn union_of_nothing_is_empty() {
        assert!(union(&[]).is_empty());
        assert!(union(&[&[], &[]]).is_empty());
    }

    #[test]
    fn a_vm_fetch_always_carries_host_names() {
        // Not a fetch (no server here), but the rule it encodes: every
        // VM-derived sheet resolves `runtime.host`, so "name" must survive into
        // the host property set even when no sheet asked for host properties.
        assert_eq!(union(&[&[], &["name"]]), vec!["name"]);
    }

    #[test]
    fn a_snapshot_derives_host_names_from_its_hosts() {
        let snap = InventorySnapshot::from_parts(
            Vec::new(),
            vec![
                test_support::host("host-1", &[("name", "esx1.example.com")]),
                test_support::host("host-2", &[("name", "esx2.example.com")]),
            ],
        );
        assert_eq!(snap.host_names.get("host-1").map(String::as_str), Some("esx1.example.com"));
        assert_eq!(snap.host_names.get("host-2").map(String::as_str), Some("esx2.example.com"));
        assert_eq!(snap.host_names.len(), 2);
    }

    /// The path index walked over the captured inventory tree.
    ///
    /// vim25 gives `parent` as `<val type="Folder" xsi:type="ManagedObjectReference">`:
    /// `xsi:type` only says "this is a reference", so the walk reads the plain
    /// `type` attribute. Inferring the type from the moref prefix would work
    /// until it did not.
    #[test]
    fn a_vm_resolves_its_datacenter_by_walking_parent_folders() {
        let snap = test_support::captured_snapshot();
        // appliance01 sits directly in the datacenter's "vm" folder.
        let vm = test_support::captured(test_support::VM_MULTI_DISK);
        assert_eq!(snap.paths.datacenter_of(&vm.moref).as_deref(), Some("datacenter01"));
        assert_eq!(snap.paths.folder_of(&vm.moref).as_deref(), Some("vm"));
    }

    /// More than one hop: this VM is in a folder nested under "vm".
    #[test]
    fn a_nested_folder_still_reaches_the_datacenter() {
        let snap = test_support::captured_snapshot();
        let vm = test_support::captured(test_support::VM_SNAPSHOTS);
        assert_eq!(snap.paths.folder_of(&vm.moref).as_deref(), Some("ESX Agents"));
        assert_eq!(snap.paths.datacenter_of(&vm.moref).as_deref(), Some("datacenter01"));
    }

    /// A host reaches its datacenter through cluster -> host folder ->
    /// datacenter, a different branch of the tree than a VM's folder path.
    #[test]
    fn a_host_resolves_its_cluster_and_datacenter() {
        let snap = test_support::captured_snapshot();
        let host = test_support::captured(test_support::HOST_FULL);
        assert_eq!(snap.paths.cluster_of_host(&host.moref).as_deref(), Some("cluster01"));
        assert_eq!(snap.paths.datacenter_of(&host.moref).as_deref(), Some("datacenter01"));
    }

    /// A VM reaches its cluster through the host it runs on, not through its
    /// folder. Only one captured VM is on the captured host; the others name a
    /// host absent from the corpus and must report nothing rather than guess.
    #[test]
    fn a_vm_reaches_its_cluster_through_its_host() {
        let snap = test_support::captured_snapshot();
        let on_host = test_support::captured(test_support::VM_SNAPSHOTS);
        assert_eq!(snap.paths.cluster_of_vm(&on_host.moref).as_deref(), Some("cluster01"));

        let elsewhere = test_support::captured(test_support::VM_MULTI_DISK);
        assert_eq!(snap.paths.cluster_of_vm(&elsewhere.moref), None);
    }

    /// An unknown moref yields nothing rather than panicking or inventing a
    /// location.
    #[test]
    fn an_unknown_moref_has_no_location() {
        let snap = test_support::captured_snapshot();
        assert_eq!(snap.paths.datacenter_of("vm-does-not-exist"), None);
        assert_eq!(snap.paths.folder_of("vm-does-not-exist"), None);
        assert_eq!(snap.paths.cluster_of_host("host-does-not-exist"), None);
    }

    /// The union is what decides how many properties one export asks for. If a
    /// sheet's set stops being merged, the export silently loses columns.
    #[test]
    fn every_registered_sheet_contributes_its_props_to_the_union() {
        let vm_sets: Vec<&[&'static str]> = crate::data::SHEETS
            .iter()
            .flat_map(|s| s.vm_props.iter().copied())
            .collect();
        let merged = union(&vm_sets);

        for spec in crate::data::SHEETS {
            for set in spec.vm_props {
                for prop in *set {
                    assert!(
                        merged.contains(prop),
                        "{} asks for {prop}, which the union dropped",
                        spec.name
                    );
                }
            }
        }
    }
}
