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
| The other 21 sheets | | 0 | Missing |

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
| Inventory path index: Datacenter, Cluster, Folder | Cross-cutting columns on ~20 sheets, appended generically the way `VI SDK Server` already is (`data/mod.rs`). Needs `parent` walks over `Folder`, `Datacenter`, `ComputeResource`, `ClusterComputeResource`, which is a different query shape than the flat reads used so far | M | Not started (lab now available) |
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

- **Inventory walks per export dropped from 10 to 2.** One `VirtualMachine`
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

### Phase 2: host and network sheets

Five come out of the host snapshot (`config.storageDevice`, `config.network`):
`vHBA` (11), `vNIC` (12), `vSwitch` (21), `vPort` (20), `vSC_VMK` (13).

Two need new object types, but still plain flat retrieves:
`dvSwitch` (27) via `DistributedVirtualSwitch`, `dvPort` (38) via
`DistributedVirtualPortgroup`.

Takes the app to 20 sheets. Size: M.

### Phase 3: infrastructure sheets

| Sheet | Source | Note |
|---|---|---|
| `vCluster` (32) | `ClusterComputeResource` | `topology.rs` already retrieves this type |
| `vDatastore` (27) | `Datastore` | `topology.rs` already retrieves this type |
| `vRP` (46) | `ResourcePool` | New type, flat read |
| `vSource` (12) | `ServiceContent.about` | Already fetched by `test_connection` (`lib.rs:36`); it is a direct call, not a ContainerView |
| `vLicense` (8) | `LicenseManager` | Direct moref call, not a ContainerView; `retrieve()` cannot express it as written |

Takes the app to 25 sheets. Size: M. `vSource` is close to free.

### Phase 4: column depth on what already exists

Flat property additions, gated on extending
`docs/VCENTER-PROPERTY-REFERENCE.md` (89 verified paths today, parity needs more).

vInfo 25 to 89, vHost 49 to 70, vDisk 25 to 40, vSnapshot 13 to 22.

Size: M, and it parallelises well across people because the sheets do not
interact.

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
   `vcsa91.vcrocs.local`, the original author's lab, which does not resolve here.

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
