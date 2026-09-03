# RVTools Parity Plan

Goal: bring STTools to feature parity with RVTools.

This plan is derived from `docs/RVTOOLS-SHEETS-AND-COLUMNS.md` (the column spec),
read against the code actually in this repo as of the fork. It is a roadmap, not
a schedule: sizes are relative, and every sheet still has to satisfy the ground
rule in `CLAUDE.md`, namely that no vCenter property path gets written without
being queried live first.

A visual version of this plan, with per-sheet coverage across all 27 sheets,
is in `docs/parity-roadmap.html`. Open it in a browser.

---

## 1. Where the repo stands

Six of RVTools' 27 sheets exist, five of them as UI sheets and one export-only.

| Sheet | RVTools cols | STTools cols | State |
|---|---:|---:|---|
| vInfo | 89 | 25 | Partial |
| vHost | 70 | 49 | Partial |
| vDisk | 40 | 25 | Partial |
| vSnapshot | 22 | 13 | Partial |
| vHealth | 3 | 3 (5 of 7 checks) | Partial |
| vMetaData | 4 | 4 (export only) | Done |
| vFileInfo | 5 | 0 | Missing |

Missing entirely: vCPU, vMemory, vPartition, vNetwork, vCD, vUSB, vTools,
vSource, vRP, vCluster, vHBA, vNIC, vSwitch, vPort, dvSwitch, dvPort, vSC_VMK,
vDatastore, vMultiPath, vLicense, vFileInfo.

Two things are already at parity and should not be re-litigated:

- **xlsx formatting.** Fonts, number formats, freeze panes, autofilter, sheet
  order and the `True`/`False` text booleans all match a real RVTools export.
  See the format table in `docs/RVTOOLS-SHEETS-AND-COLUMNS.md`.
- **Sheet plumbing.** The frontend is generic: it calls `list_sheets` and renders
  whatever `fetch_sheet` returns. `export.rs` already carries the full 27-name
  `RVTOOLS_SHEET_ORDER` and skips what does not exist yet. Adding a sheet costs
  one module plus one line in `list_sheets` and one in `fetch_all_tables`
  (`src-tauri/src/lib.rs:52`, `:83`). No UI work per sheet.

STTools is also already **ahead** of RVTools in three places, and none of this is
parity work: multi-vCenter aggregation in one view, the Insights dashboard, and
the HTML topology report.

---

## 2. The thing to fix before adding any sheet

Every sheet today runs its own full inventory walk. `vinfo`, `vdisk`,
`vsnapshot` and `vhealth` each call `retrieve("VirtualMachine", ...)`
independently, and `common.rs` adds two more walks for host names and per-host VM
totals. An export therefore does ten full inventory passes for five sheets.
That number was measured live on 2026-09-03 by instrumenting
`SoapClient::retrieve`, not estimated: each sheet pays a `HostSystem` name walk
plus its own `VirtualMachine` walk, and vHost pays two.

Ten of the 27 sheets are derived from `VirtualMachine` properties. Adding them
the current way means ten more full walks per export, against an API whose
sessions already need careful management. **This is the single decision that
governs how expensive the rest of the plan is.**

The fix: fetch once per vCenter per export, into a snapshot that sheets read from.

```
InventorySnapshot {
    vms:         Vec<ManagedObject>,   // union of every sheet's VM property set
    hosts:       Vec<ManagedObject>,   // union of every sheet's host property set
    datastores, clusters, resource_pools, dvswitches, ...
}
```

Each sheet becomes a pure function `fn rows(&InventorySnapshot) -> Vec<Vec<Cell>>`.
Two consequences beyond speed:

1. Sheets become unit-testable against one captured XML fixture, with no live
   vCenter. Today only `vhealth` (8 tests) and `vsnapshot` (1 test) have any.
2. `fetch_sheet` for a single sheet can still request a narrow snapshot, so the
   interactive path does not pay for the export path.

---

## 3. Phases

### Phase 0: foundations

| Item | Why | Size | State |
|---|---|---|---|
| `InventorySnapshot` and sheets as pure functions over it | Sets the marginal cost of the remaining 21 sheets | L | **Done** |
| Test harness for sheets without a vCenter | Every later sheet ships with a parse test instead of a live-only check | M | **Done** |
| Inventory path index: Datacenter, Cluster, Folder | Cross-cutting columns on ~20 sheets, appended generically the way `VI SDK Server` already is (`data/mod.rs`). Needs `parent` walks over `Folder`, `Datacenter`, `ComputeResource`, `ClusterComputeResource`, which is a different query shape than the flat reads used so far | M | **Done** |
| Real captured XML fixtures per object type | Replaces the hand-written fragments the tests use today with real responses | M | **Done** |

Phase 0 adds no sheets. It is still the right first move: the path index alone
adds three columns to twenty sheets, and doing it after the sheets exist means
editing twenty modules instead of one.

### What landed

`src-tauri/src/data/snapshot.rs` holds `InventorySnapshot`, which is fetched once
per vCenter with the union of the requested sheets' property sets, and
`SheetSpec`, which is what a sheet now is:

```rust
pub struct SheetSpec {
    pub name: &'static str,
    pub columns: fn() -> Vec<Column>,
    pub vm_props: &'static [&'static [&'static str]],
    pub host_props: &'static [&'static [&'static str]],
    pub rows: fn(&InventorySnapshot) -> Result<Vec<Vec<Cell>>, String>,
}
```

Consequences:

- **Inventory walks per export dropped from 10 to 3.** One `VirtualMachine`
  retrieve and one `HostSystem` retrieve now serve all five sheets. Adding the
  seven Phase 1 sheets adds zero walks: they widen the same property union.
- **`data::SHEETS` is the single registry.** `list_sheets`, `fetch_sheet` and the
  export all drive off it, so a new sheet is one module plus one line. `lib.rs`
  no longer has a per-sheet match arm.
- **A single sheet fetches only its own properties.** Opening one tab does not
  pay for the export's full union.
- **Property sets compose.** A sheet declares `&[VM_CONTEXT_PROPS, VM_PROPS]`
  rather than restating the shared context, so there is one definition of it.
- **Sheets are pure, so they are testable.** `snapshot::test_support` builds
  `ManagedObject`s from XML fragments and assembles a snapshot; tests look up
  columns by RVTools label rather than by index, so a new column cannot silently
  shift them. Test count went from 9 to 23.

### Verified against a live vCenter, 2026-09-03

Run against `vcf-mgmt-vc91.vcf.soultec.lab` (vCenter 9.1.0.0300 build 25629530;
3 hosts, 161 VMs incl. 7 templates). Both commits were built and run on the same
vCenter minutes apart, via `examples/parity_probe` (the five per-sheet fetchers,
which is exactly what `fetch_all_tables` did at cf626b8) and
`examples/union_probe` (the shared-snapshot path).

| Sheet | vCenter (derived from raw XML) | cf626b8 | 1ef59b3 per-sheet | 1ef59b3 shared snapshot |
|---|---:|---:|---:|---:|
| vInfo | 161 | 161 | 161 | 161 |
| vHost | 3 | 3 | 3 | 3 |
| vDisk | 345 | 345 | 345 | 345 |
| vSnapshot | 3 | 3 | 3 | 3 |
| vHealth | 166 | 166 | 166 | 166 |

The first column is not the app: it was derived independently from captured
`RetrievePropertiesEx` responses, replicating each sheet's row logic. So the two
builds agree with each other *and* with vCenter.

Comparison went further than row counts — every cell was diffed, joining rows on
a natural key. All five sheets are identical in all three pairings
(old/new, new-per-sheet/new-shared, old/new-shared) except for live counters
(`CPU Usage (%)`, `Memory Usage (%)`, `CPU usage %`), which drift between any two
runs. Column names and counts are identical, no sheet emitted a warning, the
xlsx exports match sheet-for-sheet and row-for-row, and all 23 unit tests pass.

Caveat worth keeping: this lab runs vSphere Supervisor, so ephemeral Kubernetes
pod VMs appear and disappear between runs. Comparisons were bracketed to
distinguish that churn from a code difference; the only key-level differences
seen were such VMs, never a value difference on a VM present in both runs.

Still unverified: the GUI itself has not been run on Windows — the fetch and
export paths were driven through the library API, not the Tauri window.

### Inventory path index, 2026-09-03

`Datacenter`, `Cluster` and `Folder` are now appended generically, the way
`VI SDK Server` already was, so the ~20 sheets still to be written inherit them
for free.

The query shape was confirmed live before any property path was written.
`parent` comes back as
`<val type="Folder" xsi:type="ManagedObjectReference">group-v4</val>` — `xsi:type`
only says "this is a reference", while the plain `type` attribute carries the
managed-object type the walk needs. Reading that beats inferring a type from the
moref prefix, which would work right up until it did not.

Two facts made this cheaper than the plan assumed:

- **`CreateContainerView` takes a repeating `<type>`,** and one
  `RetrievePropertiesEx` can carry one `propSet` per type against that single
  view. Folder + Datacenter + ComputeResource therefore cost **one** walk, not
  three. Measured: a full five-sheet export now does **3** walks (HostSystem,
  VirtualMachine, containers), up from 2.
- **A `ComputeResource` view also returns `ClusterComputeResource`,** its
  subclass, so clusters need no separate query.

The tree is not uniform, and the code follows it rather than flattening it: a VM
reaches its **folder** through `parent`, but its **cluster** through
`runtime.host` → the host's `parent`, because folders and compute live in
separate branches of the inventory. A host outside a cluster still has a
`ComputeResource` parent, but that is a container vSphere invents rather than a
cluster anyone named, so `Cluster` is left empty rather than inventing one.

Column placement follows RVTools rather than being applied uniformly: VM sheets
get Datacenter, Cluster and Folder; vHost gets Datacenter and Cluster but no
Folder, since a host's folder is the datacenter's `host` folder that RVTools
does not show; vHealth gets none, being three columns wide. `SheetSpec` declares
which via `RowSource`, and `Table::extend_from` does the work once.

Verified live: all three columns populate on every row (161 vInfo, 345 vDisk,
3 vSnapshot, 3 vHost), with real folder names — `vSpherePods`, `vm`,
`vcf-management-services`, `ESX Agents` — and row counts unchanged.

### Captured fixtures, 2026-09-03

`src-tauri/src/data/fixtures/` holds five real `RetrievePropertiesEx`
`<objects>` elements — four VMs and one host — sanitised so the public repo
carries no lab identifiers while element names, `xsi:type`, nesting and ordering
stay byte-identical to what vCenter sent. `snapshot::test_support::captured_snapshot`
assembles them into a snapshot, and every sheet now has tests that run over real
responses. Test count went from 23 to 46.

The payoff was immediate: an assertion written from imagination said a
snapshot's `state` matches the VM's current power state. The capture disagreed —
`state` records the VM's state when the snapshot was taken, so a running VM
carries a `poweredOff` snapshot. A hand-written fixture would have encoded the
wrong meaning and passed.

Five paths stay synthetic because the lab cannot produce them: a host without
NTP, nested `childSnapshotList` snapshots, `vCLS-` VMs, a VM with no name, and
RDM-backed disks. `src-tauri/src/data/fixtures/README.md` records that, so a
green suite is not mistaken for coverage of those cases.

### Phase 1: the VM-derived sheets

All seven read from the same VM snapshot. No new query pattern, no new object
type. This is the cheapest large block of parity in the whole plan.

`vCPU` (30 cols), `vMemory` (34), `vTools` (30), `vNetwork` (27),
`vPartition` (22), `vCD` (21), `vUSB` (26).

Takes the app from 6 sheets to 13. Size: M in total, and mostly S per sheet once
Phase 0 exists.

### Phase 1 landed, 2026-09-03

All seven sheets exist: vCPU, vMemory, vTools, vNetwork, vPartition, vCD, vUSB.
The app is at **13 of 27 sheets**, and the promise that they would add no
inventory walks held — measured, not assumed. A full 12-sheet export does the
same 3 walks a 5-sheet export did; only the VM property union grew, 33 → 67.

Live row counts: vInfo/vCPU/vMemory/vTools 162 each, vDisk 345, vPartition 791,
vNetwork 234, vCD 23, vUSB 1, vSnapshot 5, vHost 3, vHealth 169.

Three columns are deliberately absent rather than shipped empty, each because a
live query said so: vMemory's `Overhead` (`runtime.memoryOverhead` returned for
no VM), vNetwork's `Switch` (needs `DistributedVirtualSwitch`, which is Phase 2)
and vTools' `Required Version` (no observed source property).

**Two shapes the lab could not test were created rather than assumed.** It had
no nested snapshot and no `VirtualUSB` device, so `sttools-fixture-01` was built
to carry both. That immediately caught a real defect: a `VirtualUSB` reports
`connected` on the device itself, not inside the `connectable` block that
CD-ROMs and NICs use, so the first implementation produced an empty cell and no
error. Hand-written XML would have been written to match the wrong code.

Test count 51 → 80.

### Phase 2: host and network sheets

Five come out of the host snapshot (`config.storageDevice`, `config.network`):
`vHBA` (11), `vNIC` (12), `vSwitch` (21), `vPort` (20), `vSC_VMK` (13).

Two need new object types, but still plain flat retrieves:
`dvSwitch` (27) via `DistributedVirtualSwitch`, `dvPort` (38) via
`DistributedVirtualPortgroup`.

Takes the app to 20 sheets. Size: M. **Done** — see below.

### Phase 2 landed, 2026-09-03

All seven sheets exist: vHBA, vNIC, vSwitch, vPort, vSC_VMK, dvSwitch, dvPort.
The app is at **20 of 27 sheets**.

Walks went 3 to 5. The five host sheets add none — they widen the existing
HostSystem fetch (41 to 47 properties). The two extra are the new object types,
which genuinely need their own queries: a `DistributedVirtualSwitch` and a
`DistributedVirtualPortgroup` carry different properties, so unlike the
container walk they cannot share one propSet.

`DistributedVirtualSwitch` also joined the container walk (name and parent
only), because dvPort references its switch by moref and a `Network` view does
**not** return switches — only portgroups, which are a Network subclass. Without
that, dvPort's Switch column would have shown `dvs-20`.

Live row counts: vHBA 3, vNIC 18, vSwitch 1, vPort 1, dvSwitch 1, dvPort 60,
vSC_VMK 18.

**vSwitch and vPort had nothing to parse**, because these hosts run entirely on
a distributed switch and both properties returned empty arrays. Rather than ship
two sheets verified only by hand-written XML, an isolated standard switch and
port group were created on one host (`sttools-vSwitch` / `sttools-pg`, no
uplinks, see `docs/LAB-ENVIRONMENT.md`). Both sheets now have a real row.

That immediately corrected the port-group reader. A `HostPortGroup` carries
`computedPolicy` — the effective settings, including what it inherits from the
switch — alongside `spec/policy`, which holds only what was explicitly set. The
first implementation read the spec, so a group inheriting its teaming showed an
empty `Policy`. Also learned: `numPorts` on a switch is the elastic count ESXi
allocated, not the number requested.

A host's view of a distributed switch is a `proxySwitch` and is deliberately
*not* counted as a standard switch — RVTools separates them, and merging would
double-report the same networking.

Partial columns, all explained rather than left to look like bugs: vNIC's Speed,
Duplex, Switch and Uplink port are populated for 6 of 18 NICs, because only two
per host are cabled and `linkSpeed` is sent only for a link that is up;
vSC_VMK's Port Group is 9 of 18, because the NSX `vxlan` and `hyperbus`
VMkernel ports sit on no port group.

Test count 80 to 107.

### Phase 3: infrastructure sheets

| Sheet | Source | Note |
|---|---|---|
| `vCluster` (32) | `ClusterComputeResource` | `topology.rs` already retrieves this type |
| `vDatastore` (27) | `Datastore` | `topology.rs` already retrieves this type |
| `vRP` (46) | `ResourcePool` | New type, flat read |
| `vSource` (12) | `ServiceContent.about` | Already fetched by `test_connection` (`lib.rs:36`); it is a direct call, not a ContainerView |
| `vLicense` (8) | `LicenseManager` | Direct moref call, not a ContainerView; `retrieve()` cannot express it as written |

Takes the app to 25 sheets. Size: M. `vSource` is close to free.

### Phase 3 landed and verified, 2026-09-03

vSource, vRP, vCluster, vDatastore and vLicense. The app is at **25 of 27
sheets**; only vMultiPath and vFileInfo remain, both Phase 5 blockers.

Two of these do not fit the ContainerView shape, so the SOAP client grew two
methods: `retrieve_moref` reaches an object by its own moref (LicenseManager is
a singleton in no container), and `service_content` wraps
`RetrieveServiceContent` (the vCenter is not an object in its own inventory).

Live, all 24 tables, zero warnings: vInfo/vCPU/vMemory/vTools 164 each,
vDisk 348, vPartition 793, vNetwork 236, vCD 23, vUSB 1, vSnapshot 5,
vSource 1, vRP 43, vCluster 1, vHost 3, vHBA 3, vNIC 18, vSwitch 1, vPort 1,
dvSwitch 1, dvPort 59, vSC_VMK 18, vDatastore 4, vLicense 1, vHealth 171.

`licenses` is a `LicenseManagerLicenseInfo[]`, so its elements carry the type
name — a first pass looking for the field name found zero and would have
shipped an empty sheet with no error. An evaluation licence reports `total = 0`
meaning unlimited rather than exhausted, and is passed through as stated.

A licence key is a credential, so the fixture masks it, and that rule now lives
in the sanitiser rather than in someone's memory.

### Continuous verification

`cargo run --example property_audit` re-checks every path the app declares
against a live vCenter, reading the property sets straight off `data::SHEETS`
so there is no list to keep in sync. Ground rule 1 governs the moment code is
written; this applies it continuously, because an upgrade can retire a path
that was verified perfectly well against the previous build, and a retired path
does not error — it silently empties a column.

It earned that immediately. Against 9.1.0.0300.25629530 and again against
9.1.1.0.25712839 after the VCF upgrade:

- `summary.currentEVCModeKey` returns for no host. Expected: EVC is off.
- `hardware.systemInfo.serialNumber` returned for every host on 9.1.0.0300 and
  for none on 9.1.1 — the upgrade retired it. The mapping was deliberately left
  alone while the upgrade was in flight, re-audited once 9.1.1 was stable, and
  only then changed: `Serial number` now falls back to
  `otherIdentifyingInfo/SerialNumberTag`, which reports the same value. The
  direct field is still preferred when present, since older vCenters populate
  it. Verified live on 9.1.1: the column reads `CZ20300KTX` again.

### Verified on vCenter 9.1.1, 2026-09-03

Re-run after the VCF upgrade, all 24 tables, zero warnings: vInfo/vCPU/vMemory/
vTools 164 each, vDisk 347, vPartition 802, vNetwork 236, vCD 23, vUSB 1,
vSnapshot 5, vSource 1, vRP 43, vCluster 1, vHost 3, vHBA 3, vNIC 18, vSwitch 1,
vPort 1, dvSwitch 1, dvPort 59, vSC_VMK 18, vDatastore 4, vLicense 1,
vHealth 171.

The audit found **122 properties, 120 still returning** — every
`VirtualMachine`, `DistributedVirtualSwitch`, `DistributedVirtualPortgroup` and
`ClusterComputeResource` path survived the upgrade untouched. The two that did
not are `summary.currentEVCModeKey` (EVC is off, expected) and the serial number
above.

ESXi hosts were still on 9.1.0.0200 at that point, so host remediation had not
run and `sttools-vSwitch` survived. That may change when it does.

### Phase 4: column depth on what already exists

Flat property additions, gated on extending
`docs/VCENTER-PROPERTY-REFERENCE.md` (89 verified paths today, parity needs more).

vInfo 25 to 89, vHost 49 to 70, vDisk 25 to 40, vSnapshot 13 to 22.

Size: M, and it parallelises well across people because the sheets do not
interact.

### Phase 4 landed, 2026-09-03

Column depth on the four sheets that existed before Phase 1, all against
vCenter 9.1.1:

| Sheet | Columns before | after |
|---|---:|---:|
| vInfo | 29 | 53 |
| vHost | 52 | 63 |
| vDisk | 29 | 33 |
| vSnapshot | 17 | 19 |

(Counts include the generic Datacenter / Cluster / Folder / VI SDK Server.)

No path was written from RVTools' column names alone. `probe_candidates`
bisects a list of candidate paths against a live vCenter, which matters because
vim25 **faults the entire retrieve** if one path does not exist on the type —
a single bad guess empties every sheet, not one column. It separated three
outcomes:

- **Invalid, never to be written:** `summary.guest.guestState`,
  `config.hardware.numMonitors`, `config.hardware.videoRamSizeInKB`,
  `guest.ipStack.dnsConfig.hostName`, `guest.ipStack.dnsConfig.domainName`,
  `config.vmfsDatastore`, `config.sslThumbprint`. All look plausible; none
  exist.
- **Valid but populated on nothing here**, so not claimed as columns:
  `runtime.memoryOverhead` (which confirms dropping vMemory's `Overhead` was
  right), `config.cpuAffinity.affinitySet`,
  `config.scheduledHardwareUpgradeInfo.versionKey`, `parentVApp`,
  `summary.tpmAttestation.status`, `hardware.biosInfo.vendor`,
  `hardware.biosInfo.firmwareMajorRelease`,
  `config.consoleReservation.serviceConsoleReserved`.
- **Valid and populated:** everything now shipped.

`ResourcePool` joined the container walk, so vInfo's `Resource pool` shows a
name rather than a moref. Walk count is unchanged.

Verified live: every new column carries data, none is empty. The partial ones
are all explainable — `PowerOn` 98/164 (running VMs only), `DAS protection`
71/164 (HA-protected VMs), `Resource pool` 156/164 (templates have none).

Still short of RVTools' full 89 on vInfo. What remains is mostly per-NIC
`Network #1..#8` columns, FT detail this lab does not use, and cluster-rule
membership, which needs `ClusterComputeResource` rule objects rather than VM
properties.

### Phase 5: the two hard blockers

| Item | Needs | Unlocks |
|---|---|---|
| Datastore file browsing | `HostDatastoreBrowser` + `SearchDatastoreSubFolders` | `vFileInfo` (5 cols) and the vHealth `Zombie` check |
| Storage path enumeration | `HostMultipathInfo` / `config.storageDevice.multipathInfo` + `scsiLun` | `vMultiPath` (32 cols) |

Both are L, and the datastore walk is genuinely expensive at runtime, so it wants
to be opt-in rather than part of every export.

The seventh vHealth check, `Performance tip`, has no determinable trigger from
the reference export. Either reverse-engineer it or ship 6 of 7 with a documented
gap. Do not invent a trigger.

Completing Phase 5 reaches 27 of 27 sheets.

### vMultiPath landed, 2026-09-03 — and it was not a blocker

The plan sized this as an L, on the belief it needed `HostMultipathInfo` from an
API area the app could not reach. It did not. `config.storageDevice.multipathInfo`
and `config.storageDevice.scsiLun` both come off the `HostSystem` fetch that
vHBA and vNIC already do, so the sheet **adds no inventory walk at all**. It was
one module, like every sheet since Phase 1.

The assumption was never tested until `property_audit` was pointed at the host
properties and both came back 3/3. Worth remembering the next time a plan calls
something a blocker: the cost of checking was one query.

The two properties are shaped differently, and the pair is a compact example of
the rule in `CLAUDE.md`. `scsiLun` is a top-level array, so its elements carry
the declared type name — `<ScsiLun xsi:type="HostScsiDisk">`. `multipathInfo` is
a single object, so the LUNs beneath it repeat the *field* name `<lun>`, and each
LUN's paths repeat `<path>`. Reading either the wrong way yields no rows and no
error.

A row is a path, not a device: the sheet exists to show that a LUN is reachable
more than one way and which way is live. Live: 18 rows across 3 hosts, every
column populated. The lab's disks are local SAS with one path each, so `Working
path` is true throughout — correct, but not much of a test of the multi-path
case this sheet is named for.

**26 of 27 sheets.** Only `vFileInfo` remains, and that one is a real blocker:
it needs `HostDatastoreBrowser` and `SearchDatastoreSubFolders`, a genuinely
different API area, and the walk is expensive enough that it should be opt-in
rather than part of every export. It also unlocks vHealth's `Zombie` check.

### Phase 6: app-level parity

Sheets are not the whole product. RVTools also does these, and STTools does none
of them:

- **Headless export.** RVTools' `-c ExportAll2xls` is how people schedule it. This
  is also the README's stated Linux-service goal. Needs a second binary; note
  `CLAUDE.md` already records that this breaks `cargo run` under `tauri dev`
  unless `default-run` is set, which it now is (`sttools`).
- **CSV export** per sheet, alongside xlsx.
- **Email delivery** of a finished export over SMTP.
- **Zip and password-protect** the export.
- **Grid search, filter and column sort** in the UI.
- **Connect straight to a standalone ESXi host**, not only to vCenter.
- **Custom attributes and vSphere tags** as columns.

---

## 4. Security work to carry alongside

Both of these are already named as inherited defects in `CLAUDE.md`; they should
land with the phases, not after.

1. **Stored vCenter passwords are cleartext.** `config.json` holds them as plain
   strings. `restrict_permissions` chmods the file to `0600`, but only on Unix:
   the `#[cfg(not(unix))]` arm is an empty function
   (`src-tauri/src/vcenter/config.rs:82`), so on Windows, a first-class target
   here, the file gets default ACLs. RVTools encrypts its stored passwords. Move
   to the OS credential store (Keychain, DPAPI, libsecret).
2. **If the Linux web-service mode in Phase 6 happens, authenticate it.** The
   reference implementation bound `0.0.0.0` with no auth and an endpoint that
   returned stored vCenter credentials in cleartext. Do not inherit that.

---

## 5. Prerequisites that are not code

One blocker stands in front of Phase 1, and it is not solved by writing Rust.
The second item here is settled and recorded so it stays settled.

1. **A vCenter to develop against: settled as of 2026-09-03.**
   `vcf-mgmt-vc91.vcf.soultec.lab`, a VCF 9 management domain running vSphere
   Supervisor — 3 HPE DL380 Gen10 hosts, 161 VMs, vSAN-backed.
   `docs/LAB-ENVIRONMENT.md` is re-documented against it and records the
   connection, how credentials are kept out of this public repo, and which empty
   results are expected there rather than bugs. The previous entry pointed at
   the upstream author's lab, which does not resolve here.

   Ground rule 1 in `CLAUDE.md` still holds and is now cheap to satisfy: no new
   property path gets written before it has been queried against this vCenter.
2. **Which RVTools version is the parity target: settled, it is 4.6.** The column
   spec in `docs/RVTOOLS-SHEETS-AND-COLUMNS.md` is derived from
   `reference/RVTools_export_all_2024-08-18_15.54.15.xlsx`, and that export stays
   the reference. Every sheet and column count in this plan is sized against it.
   A newer RVTools release is explicitly out of scope; if that changes, the lists
   have to be re-derived from a fresh export before any remaining phase is sized.

---

## 6. Recommended first slice

Phase 0 items 1 and 2, then all of Phase 1.

That is one architectural change plus seven sheets that share a single code path,
and it moves the app from 6 sheets to 13 while adding Datacenter, Cluster and
Folder to every sheet at once. It also produces the fixture corpus that makes
Phases 2 to 4 testable without a live vCenter for every change.
