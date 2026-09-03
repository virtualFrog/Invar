# Lab Environment

The vCenter STTools is developed and verified against. Point the app here.

Documented 2026-09-03, from live queries against the environment itself. Counts
drift as the lab changes — see [Inventory volatility](#inventory-volatility),
which matters more here than in a typical lab.

> **This repo is public.** The vCenter password is deliberately kept out of git.
> It lives in `LAB-CREDENTIALS.local.md` at the repo root, which is gitignored
> (as is `*.local.md`). Shell snippets below expect `$VC_PASS` to be set:
>
> ```bash
> export VC_HOST='vcf-mgmt-vc91.vcf.soultec.lab'
> export VC_USER='administrator@vsphere.local'
> export VC_PASS='<password from LAB-CREDENTIALS.local.md>'
> ```

---

## Connection

| | |
|---|---|
| Host | `vcf-mgmt-vc91.vcf.soultec.lab` |
| IP | `10.24.60.30` |
| Username | `administrator@vsphere.local` |
| Password | see `LAB-CREDENTIALS.local.md` (untracked) |
| Certificate | Self-signed, issued by `CN=CA, DC=vsphere, DC=local` — the HTTP client must skip verification (`skip_cert_verify: true`) |
| Product | VMware vCenter Server 9.1.0.0300, build 25629530 (`apiVersion` 9.1.0.0) |
| Instance UUID | `574cef01-7f05-4a86-bb1d-88a92804d683` |

This is the **management vCenter of a VCF 9 fleet**, and it manages the
`vcf-wld01-*` ESXi hosts. It is not the upstream author's lab, which this file
used to describe and which does not resolve from here.

### DNS gotcha

`nslookup vcf-mgmt-vc91.vcf.soultec.lab` returns **NXDOMAIN** against the
corporate AD DNS server (`stsrvad001.soultec.local`, 10.23.46.100), yet the name
resolves fine for `curl` and the app. The `vcf.soultec.lab` zone is served by a
different resolver reachable over the lab network path. **A failed `nslookup` is
not evidence the app cannot reach vCenter** — test with `curl` instead:

```bash
curl -sk -o /dev/null -w '%{remote_ip} %{http_code}\n' "https://$VC_HOST/"
# expect: 10.24.60.30 200
```

### Credential handling

Nothing here requires writing the password to disk. The repo's Cargo examples
take it from the environment:

```bash
cargo run --example verify -- vInfo      # env-var driven, nothing persisted
```

The **app** is a different story. It reads `config.json`, and on Windows that
file is written with default ACLs — see
[Security](#security-storing-the-password) below before you use the GUI.

### Smoke test

```bash
# REST login — should return {"value":"<token>"}
curl -sk -X POST -u "$VC_USER:$VC_PASS" \
  "https://$VC_HOST/rest/com/vmware/cis/session"

# SOAP: version info, needs no authentication at all
curl -sk -X POST "https://$VC_HOST/sdk" \
  -H 'Content-Type: text/xml; charset=utf-8' -H 'SOAPAction: urn:vim25/8.0' \
  --data-binary '<?xml version="1.0" encoding="UTF-8"?><soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/" xmlns:vim25="urn:vim25"><soapenv:Body><vim25:RetrieveServiceContent><vim25:_this type="ServiceInstance">ServiceInstance</vim25:_this></vim25:RetrieveServiceContent></soapenv:Body></soapenv:Envelope>' \
  | grep -o '<fullName>[^<]*</fullName>'
```

---

## What's in the environment

| | |
|---|---|
| Datacenter | `vcf-mgmt-dc01` |
| Cluster | `vcf-mgmt-cl01` (1), DRS and HA both enabled |
| ESXi hosts | 3 |
| VMs | 162 via SOAP, of which 7 are templates |
| Datastores | 4 |
| Networks | 60 (59 distributed portgroups, 1 standard) |
| Resource pools | 43 |
| Snapshots | 3 |

**VM count depends on which API you ask.** REST `/rest/vcenter/vm` reports 154;
the SOAP `ContainerView` the app uses reports **161**. The difference is exactly
the 7 templates, which REST omits and SOAP includes. RVTools counts templates,
so 161 is the number vInfo should produce. Do not "fix" a 161 against a REST-derived
154.

### Hosts

`vcf-wld01-esx01`, `vcf-wld01-esx02`, `vcf-wld01-esx03` (all `.vcf.soultec.lab`)

Identical physical boxes, and real server hardware rather than nested ESXi:

| | |
|---|---|
| Model | HPE ProLiant DL380 Gen10 |
| CPU | 2 × Intel Xeon Gold 6252 @ 2.10 GHz — 48 cores, 96 threads, HT active |
| ESXi | 9.1.0.0200, build 25557999 |
| NICs / HBAs | 6 / 1 |
| Memory tiering | DRAM only |

All three have NTP configured (`10.24.0.10`) and `ntpd` running, so they produce
**no vHealth findings** — see below.

### Datastores

| Name | Type | Capacity |
|---|---|---|
| `vcf-mgmt-cl01-ds-vsan01` | vSAN | 36.0 TB |
| `datastore1` | VMFS | 1.46 TB |
| `datastore1 (1)` | VMFS | 1.46 TB |
| `datastore1 (2)` | VMFS | 1.46 TB |

Essentially everything lives on the vSAN datastore. That single fact drives the
biggest surprise in the vHealth sheet, below.

### `sttools-fixture-01` — a VM created for testing

This VM exists only to give the test corpus two shapes the lab otherwise lacked.
It is powered off, has no disk and no NIC, and carries:

- a **nested snapshot**: `sttools-parent` with `sttools-child` beneath it, which
  is the `childSnapshotList` shape vSnapshot and vHealth flatten;
- a **`VirtualUSB` device** on a USB controller, which is what vUSB parses.

Before it existed, both code paths were exercised only by hand-written XML. It
is safe to delete — the annotation on the VM says so — but deleting it takes
vUSB back to zero rows and removes the only nested snapshot, so the fixtures in
`src-tauri/src/data/fixtures/` should be regarded as the durable record.

### Workload character

This is a **VCF 9 management domain running vSphere Supervisor**, so most VMs are
Kubernetes-related rather than classic server VMs: `SupervisorControlPlaneVM`,
`cci-ns-controller-manager-*`, `harbor-*`, `argocd-*`, Avi service engines,
`vSAN File Service Node (1..3)`. There are also names containing spaces and
parentheses, which is useful — they exercise quoting and XML escaping that a lab
of tidy names would not.

---

## Expected empty results — not bugs

Verified 2026-09-03 by inspecting raw `RetrievePropertiesEx` responses. A blank
column here is a fact about the lab, not a broken property path. **Check this
list before "fixing" an empty column.**

| Sheet | Column(s) | Rows filled | Why |
|---|---|---|---|
| vDisk | `Raw LUN ID`, `Raw Comp. Mode` | 0/345 | No RDMs exist. All 345 disks are `VirtualDiskFlatVer2BackingInfo`; not one is `RawDiskMapping*`, so `Raw` is `False` everywhere and the two RDM-only columns have nothing to report. |
| vHost | `NVMe Tier GiB` | 0/3 | Memory tiering reports `DRAM` only. No NVMe tier is configured. |
| vHost | `Current EVC` | 0/3 | EVC is not enabled on the cluster. `Max EVC` *is* populated — that asymmetry is correct. |
| vHost | `DNS Search Order` | 0/3 | No search domains configured on the hosts. |
| vInfo | `DNS Name` | 99/161 | Requires VMware Tools to report a hostname. Many Supervisor pod VMs and all powered-off VMs do not. |
| vInfo | `Primary IP Address` | 97/161 | Same cause as `DNS Name`. |
| vInfo | `CPU Usage (%)` | 79/161 | Needs `summary.runtime.maxCpuUsage`, which a powered-off VM does not report. The code deliberately leaves this empty rather than writing `0` — 0 % and "not reported" are different facts. |
| vInfo | `Annotation` | 46/161 | Most VMs simply carry no annotation. |
| vHealth | NTP / NTPD findings | 0 | All three hosts have NTP servers set and `ntpd` running. A clean result. |
| all | vCLS exclusion | never fires | **No VM in this lab is named `vCLS-*`.** The `starts_with("vCLS-")` filter in `common.rs` and `vinfo.rs` is inert here, so it is *not* exercised against the live lab — only by unit tests. Do not read a passing lab run as evidence that filter works. |
| vNetwork | `Network` | 178/234 | 56 NICs use `VirtualEthernetCardLegacyNetworkBackingInfo`, whose `deviceName` comes back empty. Their VMs do report `guest.net`, but the guest names none of those NICs either, so no source supplies a name. |
| vNetwork | `IPv4 Address` | 97/234 | Comes from `guest.net`, which needs VMware Tools. |
| vMemory | `Overhead` | column absent | `runtime.memoryOverhead` was returned for **no** VM, so the column is not implemented rather than shipped always-empty. |
| vUSB | all | 1 row | Only `sttools-fixture-01` has a `VirtualUSB` device; the lab's other USB entries are *controllers*, which are not devices and are deliberately not rows. |

### Host networking is distributed, plus one switch built for testing

Production networking here is entirely distributed: one switch,
`vcf-mgmt-cl01-vds01` (`dvs-20`), with 60 port groups. No host had a standard
vSwitch or port group at all, so RVTools' `vSwitch` and `vPort` sheets parsed
nothing.

`sttools-vSwitch` and its port group `sttools-pg` (VLAN 101) were created on
**esx01 only** to fix that. The switch is **isolated — no physical NIC is
attached** — so it carries no traffic and cannot affect existing networking.
Both sheets now have one real row each.

To remove them, on `networkSystem-12`:
`RemovePortGroup(pgName="sttools-pg")` then
`RemoveVirtualSwitch(vswitchName="sttools-vSwitch")`. Doing so returns vSwitch
and vPort to zero rows.

Two things the real switch taught, both now in the code:

- `numPorts` is the **elastic** count ESXi allocated (9216), not the 128 that
  was requested; the request survives separately under `spec/numPorts`.
- A port group's effective settings are in `computedPolicy`, not `spec/policy`.
  `sttools-pg` sets only security and inherits teaming from the switch, so
  `spec/policy` has no `nicTeaming` at all and reading it would have left the
  `Policy` column empty.

Two more partial columns worth knowing before they look like defects:

| Sheet | Column(s) | Rows filled | Why |
|---|---|---|---|
| vNIC | `Speed`, `Duplex`, `Switch`, `Uplink port` | 6/18 | Only two of each host's six NICs are cabled. `linkSpeed` is sent only for a link that is up, and an unattached NIC backs no uplink. |
| vSC_VMK | `Port Group` | 9/18 | The NSX `vxlan` and `hyperbus` VMkernel ports sit on no port group at all. |
| dvPort | `Active Uplink` | 9/60 | Most port groups inherit teaming from the switch and vCenter does not materialise `uplinkPortOrder` for them. |

### A property that came and went

`hardware.systemInfo.serialNumber` was returned by all three hosts when this
lab was first documented, and returned by **none** of them a few hours later,
while a VCF upgrade to 9.1.1 was in progress. Host connection state stayed
`connected` and `overallStatus` stayed green throughout.

vHost's `Serial number` column is therefore empty at the moment. The value
itself is not lost: it is also in
`hardware.systemInfo.otherIdentifyingInfo` under `SerialNumberTag`,
`EnclosureSerialNumberTag` and `ServiceTag`, and vHost's `Service tag` column
reads the last of those, which still works.

Nothing was changed in response. Run `cargo run --example property_audit` after
the upgrade completes; if the property is still missing then, it is a real
change worth handling, and if it is back it was transient.

### The one that looks alarming and is not

**vHealth reports `Inconsistent Foldername!` for all 161 VMs.**

The check compares the VM's name against the folder component of
`config.files.vmPathName`. On this vSAN datastore the folder is an object UUID,
never the VM name:

```
[vcf-mgmt-cl01-ds-vsan01] c7d5116a-9820-2992-c40e-9440c98fd8cc/vcfsddc91.vmx
                          ^ folder is a UUID, VM is "vcfsddc91"
```

So the check fires for every VM and always will, on any vSAN-backed inventory.
This is the code faithfully reproducing RVTools' rule, not a defect — real
RVTools behaves the same way against vSAN. It does mean **vHealth's row count here
is dominated by one check**: 166 rows = 161 Foldername + 3 Snapshot + 2 CDROM + 0 NTP/NTPD.

The 3 snapshot findings are `eam-snapshot` on the three `vSAN File Service Node`
VMs, created by ESX Agent Manager rather than by a person.

---

## Inventory volatility

**This lab does not hold still.** vSphere Supervisor creates and destroys
ephemeral pod VMs continuously. During a single verification session a VM named
`metrics-aggregator-c97c7598f-59qcn` was replaced by
`metrics-aggregator-5757665c6d-8262q`, changing vInfo by one row, vDisk by three
and vHealth by one — between two runs minutes apart.

Consequences for anyone comparing runs:

- **Bracket your comparisons.** Run A, run B, then run A again. A difference that
  appears in the A/B pair *and* the A/A pair is lab churn, not code.
- **Diff on keys, not row counts.** A changed count with a matching
  appeared/disappeared VM name is drift. A changed count with no such VM is a bug.
- Expect volatile columns to differ between any two runs regardless:
  `CPU Usage (%)`, `Memory Usage (%)` (vInfo), `CPU usage %`, `Memory usage %`
  (vHost), and the vHost VM rollups.

Observed counts sit around 161/3/345/3/166 (vInfo/vHost/vDisk/vSnapshot/vHealth)
but were seen at 160/3/342/3/165 an hour later. Both are correct.

---

## Verification harness

Two Cargo examples exist specifically for checking the app against this lab:

```bash
cargo run --example parity_probe -- out.json out.xlsx   # five per-sheet fetchers
cargo run --example union_probe  -- out.json out.xlsx   # shared-snapshot path
```

Both read the app's own `config.json`, dump every table as JSON for diffing, and
write a real xlsx. `parity_probe` uses only function signatures that are
identical at `cf626b8` and `1ef59b3`, so it compiles and runs at either commit —
that is how the Phase 0 refactor was verified. See the verification section of
`docs/PARITY-PLAN.md` for the result.

The repo's older `verify`, `export` and `concurrent` examples take `VC_HOST` /
`VC_USER` / `VC_PASS` from the environment instead, and persist nothing.

---

## Security: storing the password

`config.json` lives at `%APPDATA%\ch.soultec.sttools\config.json` on Windows and
`~/Library/Application Support/ch.soultec.sttools/config.json` on macOS. **It
stores the vCenter password in cleartext.**

`config::restrict_permissions` chmods it to `0600` — but only on Unix. The
`#[cfg(not(unix))]` arm is an empty function
(`src-tauri/src/vcenter/config.rs:82`), so on Windows the file keeps default
ACLs. Confirmed on this machine 2026-09-03:

```
NT AUTHORITY\SYSTEM        FullControl
BUILTIN\Administrators     FullControl     <-- any local admin reads the password
SOULTEC\dario.doerflinger  FullControl
```

Windows is a first-class target for this app, so treat the GUI config path as
exposing the password to every local administrator until this is fixed. Tracked
in `docs/PARITY-PLAN.md` section 4; the fix is the OS credential store (DPAPI on
Windows, Keychain, libsecret). Prefer the env-var examples for development work
that does not need the GUI.
