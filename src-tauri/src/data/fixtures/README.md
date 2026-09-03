# Captured vCenter fixtures

Real `RetrievePropertiesEx` responses, one `<objects>` element per file, used by
the sheet unit tests via `data::snapshot::test_support`.

Captured 2026-09-03 from the lab described in `docs/LAB-ENVIRONMENT.md`
(vCenter 9.1.0.0300, ESXi 9.1.0.0200, vSAN-backed).

## Why these exist

Before this, every sheet test built its own XML by hand. That only ever proves
the parser copes with shapes we already imagined — and the shapes are the hard
part of vim25. Top-level arrays are named after the field's declared *type*
(`<VirtualDevice xsi:type="VirtualDisk">`, `<VirtualMachineSnapshotTree>`),
while arrays *nested inside* a data object repeat the *field* name instead.
Getting that wrong yields zero rows and no error.

These files are what vCenter actually sent.

The value showed up immediately: a hand-written snapshot fixture would almost
certainly have set the snapshot's `state` to match the VM's current power state.
The real capture has a running VM whose snapshot records `poweredOff`, because
`state` is the VM's state *at the moment the snapshot was taken*. The assertion
written from imagination was wrong; the capture corrected it.

## Sanitisation

The repo is public, so identifying **values** were replaced. **Structure was
not touched**: element names, `xsi:type` attributes, ordering, nesting depth and
whitespace are exactly as returned.

| Replaced | With |
|---|---|
| Host / vCenter FQDNs | `esx01.lab.local`, `vcenter.lab.local` |
| Org-specific VM and object names | readable stand-ins, or `node-<hash>` |
| Domain names | `lab.local` |
| IPv4 addresses | `192.0.2.0/24` (TEST-NET-1, RFC 5737) |
| Hardware UUIDs, VM UUIDs, vSAN object UUIDs | deterministic fakes |
| Chassis serial numbers | `SERIAL-XXXXXX` |
| MAC addresses | `00:50:56:…` (VMware OUI) |
| `config.annotation` free text | a fixed placeholder |

Replacement is deterministic, so the same input value maps to the same output
everywhere and cross-references inside a file still line up.

One deliberate addition: the real response carried `xmlns:xsi` on the SOAP
envelope, which slicing out a single `<objects>` element drops. It is
re-declared on the fragment root so each file parses standalone. Nothing else
was added or removed.

Note that the hardware UUID hex-encodes the chassis serial, so a naive
find-and-replace on the serial string alone would have leaked it.

## The files

| File | Captured from | Exercises |
|---|---|---|
| `vm_multi_disk.xml` | a 6-disk appliance | vDisk: multiple controllers, `storageIOAllocation` nested `shares`, KiB→MiB conversion, non-disk devices in the same array |
| `vm_snapshots.xml` | a vSAN File Service Node | vSnapshot and vHealth snapshot findings; also a name containing spaces and parentheses |
| `vm_connected_cdrom.xml` | a Kubernetes control-plane VM | vHealth CDROM: a genuinely connected `VirtualCdrom`, alongside NICs that carry their own `connectable` block |
| `vm_template.xml` | a Windows template | `config.template = true`, powered off, `.vmtx` rather than `.vmx` |
| `host_full.xml` | an ESXi host | vHost across all 40 host properties; vHealth NTP/NTPD via the `HostService` array |
| `containers.xml` | the Folder / Datacenter / ClusterComputeResource chain | the inventory path index: Datacenter, Cluster and Folder resolution. Holds every ancestor the VM and host captures reference, up to the datacenter, so the walk runs over a complete tree. Several `<objects>` under one root, loaded with `captured_many`. |

Three of the four VMs reference `host-28`, which is deliberately *not* in the
corpus, so the unresolved-moref fallback is exercised by real data rather than a
contrived moref. Only `vm_snapshots.xml` sits on the captured host.

## What stays synthetic, and why

Some paths cannot be captured because this lab never produces them. Those tests
keep hand-written fragments, and that is not laziness:

| Case | Why no capture |
|---|---|
| Host with no NTP server / ntpd stopped | All three lab hosts are correctly configured |
| Nested snapshots (`childSnapshotList`) | No snapshot in the lab has children |
| `vCLS-` prefixed VMs | None exist; the exclusion filter never fires here |
| A VM with no `name` property | An error path vCenter does not produce on demand |
| RDM-backed disks | No raw device mappings exist in the lab |

A green run against this corpus therefore says nothing about those five paths.
`docs/LAB-ENVIRONMENT.md` lists the same gaps from the live-data side.

## Regenerating

Capture is a plain `RetrievePropertiesEx` against a live vCenter, sliced to one
`<objects>` element and passed through the sanitiser. If you re-capture, re-read
the sanitisation table above and confirm no identifying value survives before
committing — the deny-list check exists because the first attempt leaked a
truncated hostname embedded in a swap-file name.
