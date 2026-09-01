# Lab Environment

The vCenter used to develop and verify the reference implementation. Point the
new app here for development.

> **This repo is public.** The vCenter password is deliberately kept out of git.
> It lives in `LAB-CREDENTIALS.local.md` at the repo root, which is gitignored.
> Shell snippets below expect `$VC_PASS` to be set:
>
> ```bash
> export VC_PASS='<password from LAB-CREDENTIALS.local.md>'
> ```

---

## Connection

| | |
|---|---|
| Host | `https://vcsa91.vcrocs.local` |
| IP | `192.168.101.10` |
| Username | `administrator@vcrocs.local` |
| Password | see `LAB-CREDENTIALS.local.md` (untracked) |
| Certificate | Self-signed — the HTTP client must skip verification |

**Prior/alternate lab** seen in older code, likely retired: `192.168.6.195`,
user `administrator@vcrocs.local`, password held locally (note: it is shorter than the current one —
if you hit an old config, that's why).

### Smoke test

```bash
# REST login — should return {"value":"<token>"}
curl -sk -X POST -u "administrator@vcrocs.local:$VC_PASS" \
  https://vcsa91.vcrocs.local/rest/com/vmware/cis/session

# Then list hosts
TOKEN=<token>
curl -sk -H "vmware-api-session-id: $TOKEN" \
  https://vcsa91.vcrocs.local/rest/vcenter/host | python3 -m json.tool
```

If this fails from the venue, the app can't work either — have offline
screenshots ready.

---

## What's in the environment

Snapshot taken while writing this; counts drift as the lab changes.

| | |
|---|---|
| vCenter version | 9.1.0.0, build 25370922 |
| Type | vCenter Server with embedded PSC |
| ESXi hosts | 3 |
| VMs | 23 |
| Clusters | 2 |
| Datastores | 7 |
| Licensing | Evaluation |

### Hosts

`esx9-01.vcrocs.local`, `esx9-02.vcrocs.local`, `esx9-03.vcrocs.local`

Consumer-grade hardware — vendor reports as "Micro Computer (HK) Tech Limited",
models "Venus Series" / "MS-A2", Intel Core i9-13900H class CPUs. Storage
adapters are **local NVMe**, so `vHBA` rows have no WWN (that's correct, not a
bug). An `esx9-04` existed earlier and was removed.

### Clusters

`CL-01`, `CL-02`

### Datastores

| Name | Type |
|---|---|
| ESX9-01-2TB | VMFS |
| ESX9-02-4TB | VMFS |
| ESX9-03-2TB-1 | VMFS |
| ESX9-03-2TB-2 | VMFS |
| SYN-HDD | NFS |
| SYN-SSD-04 | NFS |
| SYN-SSD-05 | NFS |

### Networking

All networking is on a **distributed switch** — `DSwitch-vCROCS` (`dvs-1025`).

Portgroups: `Management`, `VL-101` (VLAN 101), `NFS`, `vMotion`, `VM Network`,
`DSwitch-vCROCS-DVUplinks-1025`.

**There are no standard vSwitches or standard port groups.** `vSwitch` and
`vPort` correctly return **zero rows** here. Don't chase that as a bug — test
those two sheets against a different environment if you need to see data.

### VMs

~23, including `OPS91` (VMware Photon OS), `lic91`, `MPB2`, `minion01`. Guest OS
values come back as enums (`VMWARE_PHOTON_64`) unless you read
`full_name.default_message`, which gives the friendly string
("VMware Photon OS (64-bit)").

vCLS VMs (names starting `vCLS-`) are vSphere-managed and filtered out by the
reference — do the same or your VM counts won't match the vSphere UI.

---

## Environment-specific behaviour to expect

Things that look like bugs here but aren't:

| Observation | Why |
|---|---|
| `vSwitch` / `vPort` empty | No standard switches — all distributed |
| `vHBA` WWN blank | Local NVMe adapters have no world-wide name |
| `vSnapshot` often empty | As of 2026-08-31 there are exactly 2: one each on `minion01` and `minion02`, both `.vmsn`-only (memoryKey `-1`, so no `.vmem`). Neither has children, so **nested snapshots are untested here** — create a snapshot of a snapshot if you need to exercise the recursion. |
| `vDisk` shows no RDMs | All 85 disks are `VirtualDiskFlatVer2BackingInfo`. `Raw`, `Raw LUN ID` and `Raw Comp. Mode` are correctly empty; they need `RawDiskMappingVer1BackingInfo`, which this lab has none of. |
| `vLicense` shows "Product Evaluation", total 0 | Eval licensing |
| Host `/hardware` REST endpoints 404 | They genuinely don't exist on this version — that's why SOAP is required |
| VM counts differ from vSphere UI | vCLS VMs filtered out |
| SOAP returns one more VM than REST | `/rest/vcenter/vm` omits templates; `VirtualMachine` over SOAP includes them. Verified 2026-08-31: 24 via REST, 25 via SOAP, the extra being `vcf-services-runtime-template-9.1.0.0.25370367`. RVTools' vInfo includes templates with `Template = True`, so the SOAP count is the correct one. |
| Host memory far larger than the box could hold (478 GiB on a mini PC) | Memory tiering is enabled. `hardware.memorySize` / `summary.hardware.memorySize` report DRAM **plus** the NVMe tier. Verified 2026-08-31: esx9-02/03 are 95.73 GiB DRAM + 382.94 GiB NVMe; esx9-01 is 63.73 + 254.94. Break the tiers out of `hardware.memoryTierInfo` (`<HostMemoryTierInfo>` with `type` of `DRAM` or `NVMe`) rather than presenting the total as physical RAM. Note `summary.hardware.memoryTieringType` does **not** exist — asking for it faults the whole query with HTTP 500. |
| `summary.currentEVCModeKey` absent on some hosts | Only set for hosts in an EVC-enabled cluster; 1 of 3 here. |
| ~300 open sessions on the lab vCenter | Mostly lab infrastructure, not your app: vCenter's own `vapi-endpoint` (~168), VCF Operations on `192.168.101.9` (~47), and govmomi checkers on `192.168.101.50`. Filter `sessionList` by your workstation's IP before concluding you are leaking. |

---

## Checking for session leaks

The single most useful diagnostic while developing. Counts `<UserSession>`
entries; a healthy app holds a couple, not hundreds.

```bash
BODY='<?xml version="1.0" encoding="UTF-8"?>
<soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/" xmlns:vim25="urn:vim25">
  <soapenv:Body><vim25:Login>
    <vim25:_this type="SessionManager">SessionManager</vim25:_this>
    <vim25:userName>administrator@vcrocs.local</vim25:userName>
    <vim25:password>$VC_PASS</vim25:password>
  </vim25:Login></soapenv:Body>
</soapenv:Envelope>'

COOKIE=$(curl -sk -i -X POST -H "Content-Type: text/xml; charset=utf-8" \
  -H "SOAPAction: urn:vim25/8.0" --data "$BODY" https://vcsa91.vcrocs.local/sdk \
  | grep -i "^set-cookie" | sed -E 's/.*(vmware_soap_session="[^"]*").*/\1/')

QUERY='<?xml version="1.0" encoding="UTF-8"?>
<soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/" xmlns:vim25="urn:vim25">
  <soapenv:Body><vim25:RetrievePropertiesEx>
    <vim25:_this type="PropertyCollector">propertyCollector</vim25:_this>
    <vim25:specSet>
      <vim25:propSet><vim25:type>SessionManager</vim25:type><vim25:pathSet>sessionList</vim25:pathSet></vim25:propSet>
      <vim25:objectSet><vim25:obj type="SessionManager">SessionManager</vim25:obj></vim25:objectSet>
    </vim25:specSet><vim25:options/>
  </vim25:RetrievePropertiesEx></soapenv:Body>
</soapenv:Envelope>'

curl -sk -X POST -H "Content-Type: text/xml; charset=utf-8" -H "SOAPAction: urn:vim25/8.0" \
  -H "Cookie: $COOKIE" --data "$QUERY" https://vcsa91.vcrocs.local/sdk \
  | grep -o "<UserSession" | wc -l
```

---

## RVTools reference export

`/Users/dalehassinger/Documents/GitHub/PS-TAM-Lab/RVTools/RVTools_export_all_2024-08-18_15.54.15.xlsx`

From a **different, larger environment** (`vcsa8x.corp.local`, vCenter 8) — useful
for column names and formatting, but its data won't match this lab. Copy it into
the new repo so the team has it without depending on that path.

Formatting to match: header row is **Verdana 9pt bold, white on black**, freeze
panes at `B2`, autofilter across the data range.
