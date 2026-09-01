# RVTools Sheets & Columns Reference

Every sheet and column from a real RVTools export, cross-referenced against what
the reference implementation actually sources from vCenter.

Use this to (a) name your sheets and columns exactly as RVTools does, and
(b) know upfront which columns are cheap and which are expensive.

**Column counts below exclude RVTools boilerplate** repeated on nearly every
sheet: `VI SDK Server`, `VI SDK UUID`, `VI SDK Server type`, `VI SDK API Version`,
`com.vrlcm.snapshot`, `Internal Sort Column`, `Object ID`. Also common across
most sheets and omitted from per-sheet notes: `Datacenter`, `Cluster`, `Host`,
`Folder`, `Annotation`, `VM ID`, `VM UUID`, `Powerstate`, `Template`,
`SRM Placeholder`, `OS according to the configuration file`,
`OS according to the VMware Tools`.

`Datacenter` and `Cluster` are **not** implemented in the reference — they need
walking the inventory tree from each object up to its parent, which is a
different query pattern than a flat property read. Budget for that if you want them.

---

## Coverage summary

| Sheet | RVTools cols | Reference impl | Notes |
|---|---:|---:|---|
| vInfo | 89 | 24 (27%) | |
| vCPU | 30 | 18 (60%) | |
| vMemory | 34 | 22 (65%) | |
| vDisk | 40 | 19 (48%) | |
| vPartition | 22 | 10 (45%) | |
| vNetwork | 27 | 13 (48%) | |
| vCD | 21 | 9 (43%) | |
| vUSB | 26 | 9 (35%) | |
| vSnapshot | 22 | 9 (41%) | |
| vTools | 30 | 11 (37%) | |
| vSource | 12 | 12 (100%) | |
| vRP | 46 | 12 (26%) | |
| vCluster | 32 | 2 (6%) | |
| vHost | 70 | 30 (43%) | |
| vHBA | 11 | 9 (82%) | |
| vNIC | 12 | 7 (58%) | |
| vSwitch | 21 | 8 (38%) | |
| vPort | 20 | 7 (35%) | |
| dvSwitch | 27 | 7 (26%) | |
| dvPort | 38 | 12 (32%) | |
| vSC_VMK | 13 | 9 (69%) | |
| vDatastore | 27 | 5 (19%) | |
| vMultiPath | 32 | — | Not implemented: Needs HostMultipathInfo / storage path enumeration |
| vLicense | 8 | 5 (62%) | |
| vFileInfo | 5 | — | Not implemented: Needs HostDatastoreBrowser + SearchDatastoreSubFolders (datastore file-tree walking) |
| vHealth | 3 | 5 of 7 checks | Not alarms — RVTools' own computed checks. Zombie and Performance tip not implemented; see below |
| vMetaData | 4 | 4 (100%) | |

A realistic hackathon target is 6–8 sheets. Highest demo value per unit of
effort: **vHost, vInfo, vCPU, vMemory, vDatastore, vSnapshot** — plus a
performance/gauge view, which RVTools doesn't have at all.

---

## xlsx format parity

Read out of `reference/RVTools_export_all_2024-08-18_15.54.15.xlsx` (RVTools 4.6)
and reproduced by `src-tauri/src/export.rs`. Verified 2026-08-31 by generating an
export from the lab and reading it back with openpyxl.

| Element | RVTools | Ours |
|---|---|---|
| Header row | Verdana 9pt bold, `FFFFFFFF` on solid `FF000000`, left aligned | same |
| Body text | Verdana 9pt, general alignment | same |
| Integers | `#,##0`, right aligned | same |
| Decimals | `#,##0.00`, right aligned | same |
| Dates | real Excel serials, `yyyy/MM/dd HH:mm:ss` | same |
| Booleans | the text `True` / `False` — **not** Excel booleans | same |
| Freeze panes | `B2` (first row *and* first column) | same |
| AutoFilter | `A1` to the last cell, header included | same |
| Default row height | 15 | same |
| Sheet order | fixed RVTools order | same order, sheets we lack are skipped |
| Column widths | autofit to content | autofit to content (differs by data, as RVTools' would) |

RVTools also styles a column uniformly rather than per cell, so the format is
chosen from the whole column: a `Number` column is `#,##0` only if every value
is integral, and a `Text` column becomes a date column only if every non-empty
value parses as RFC 3339.

Two deliberate differences:

- **`vMetaData` keeps RVTools' four column names** (`RVTools major version`,
  `RVTools version`, `xlsx creation datetime`, `Server`) so RVTools-aware
  tooling still parses the file, but the values name *this* tool. Writing an
  RVTools version number would misstate where the data came from.
- The freeze pane is emitted as `state="frozen"` where RVTools writes
  `state="frozenSplit"`. Both are frozen panes at `B2` in Excel;
  `rust_xlsxwriter` does not expose the split variant.

Timestamps are written as vCenter reports them (UTC), not converted to local
time — converting would silently relabel every date in the export.

---

## Per-sheet detail

For each sheet: the full RVTools column list, then the subset the reference
implements with the JSON field name it uses.


### vInfo

**RVTools columns:**

```
VM, Powerstate, Template, SRM Placeholder, Config status, DNS Name, Connection state, Guest state, Heartbeat, Consolidation Needed, PowerOn, Suspended To Memory, Suspend time, Suspend Interval, Creation date, Change Version, CPUs, Overall Cpu Readiness, Memory, Active Memory, NICs, Disks, Total disk capacity MiB, Fixed Passthru HotPlug, min Required EVC Mode Key, Latency Sensitivity, Op Notification Timeout, EnableUUID, CBT, Primary IP Address, Network #1, Network #2, Network #3, Network #4, Network #5, Network #6, Network #7, Network #8, Num Monitors, Video Ram KiB, Resource pool, Folder ID, Folder, vApp, DAS protection, FT State, FT Role, FT Latency, FT Bandwidth, FT Sec. Latency, Vm Failover In Progress, Provisioned MiB, In Use MiB, Unshared MiB, HA Restart Priority, HA Isolation Response, HA VM Monitoring, Cluster rule(s), Cluster rule name(s), Boot Required, Boot delay, Boot retry delay, Boot retry enabled, Boot BIOS setup, Reboot PowerOff, EFI Secure boot, Firmware, HW version, HW upgrade status, HW upgrade policy, HW target, Path, Log directory, Snapshot directory, Suspend directory, Annotation, Application, LOCATION, Reboot, Datacenter, Cluster, Host, OS according to the configuration file, OS according to the VMware Tools, Customization Info, Guest Detailed Data, VM ID, SMBIOS UUID, VM UUID
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| VM | `name` |
| CPUs | `cpu` |
| Memory | `memory` |
| Powerstate | `power_state` |
| Primary IP Address | `ipAddress` |
| OS according to the VMware Tools | `guestOs` |
| DNS Name | `dnsName` |
| CPU Usage (%) | `cpuUsagePercent` |
| Memory Usage (%) | `memoryUsagePercent` |
| Host | `host` |
| Creation date | `creationDate` |
| HW version | `hwVersion` |
| Annotation | `annotation` |
| Template | `template` |
| Firmware | `firmware` |
| EFI Secure boot | `efiSecureBoot` |
| Cores p/s | `coresPerSocket` |
| Provisioned GiB | `provisionedGiB` |
| In Use GiB | `inUseGiB` |
| Path | `vmPath` |
| Change Version | `changeVersion` |
| VM UUID | `vmUuid` |
| Tools Version Status | `toolsVersionStatus` |
| Tools Running Status | `toolsRunningStatus` |


### vCPU

**RVTools columns:**

```
VM, Powerstate, Template, SRM Placeholder, CPUs, Sockets, Cores p/s, Max, Overall, Level, Shares, Reservation, Entitlement, DRS Entitlement, Limit, Hot Add, Hot Remove, Numa Hotadd Exposed, Annotation, Application, LOCATION, Reboot, Datacenter, Cluster, Host, Folder, OS according to the configuration file, OS according to the VMware Tools, VM ID, VM UUID
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| VM | `name` |
| Powerstate | `power_state` |
| Template | `template` |
| CPUs | `cpu` |
| Sockets | `sockets` |
| Cores p/s | `coresPerSocket` |
| Max | `maxCpuMhz` |
| Overall | `overallCpuMhz` |
| Level | `level` |
| Shares | `shares` |
| Reservation | `reservation` |
| Entitlement | `entitlement` |
| DRS Entitlement | `drsEntitlement` |
| Limit | `cpuLimit` |
| Hot Add | `hotAdd` |
| Hot Remove | `hotRemove` |
| Annotation | `annotation` |
| Host | `host` |


### vMemory

**RVTools columns:**

```
VM, Powerstate, Template, SRM Placeholder, Size MiB, Memory Reservation Locked To Max, Overhead, Max, Consumed, Consumed Overhead, Private, Shared, Swapped, Ballooned, Active, Entitlement, DRS Entitlement, Level, Shares, Reservation, Limit, Hot Add, Annotation, Application, LOCATION, Reboot, Datacenter, Cluster, Host, Folder, OS according to the configuration file, OS according to the VMware Tools, VM ID, VM UUID
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| VM | `name` |
| Powerstate | `power_state` |
| Template | `template` |
| Size MiB | `memory` |
| Memory Reservation Locked To Max | `memReservationLockedToMax` |
| Overhead | `memOverheadMiB` |
| Consumed | `memConsumedMiB` |
| Consumed Overhead | `memConsumedOverheadMiB` |
| Private | `memPrivateMiB` |
| Shared | `memSharedMiB` |
| Swapped | `memSwappedMiB` |
| Ballooned | `memBalloonedMiB` |
| Active | `memActiveMiB` |
| Entitlement | `memEntitlementMiB` |
| DRS Entitlement | `memDrsEntitlementMiB` |
| Level | `memLevel` |
| Shares | `memShares` |
| Reservation | `memReservationMiB` |
| Limit | `memLimitMiB` |
| Hot Add | `memHotAdd` |
| Annotation | `annotation` |
| Host | `host` |


### vDisk

**RVTools columns:**

```
VM, Powerstate, Template, SRM Placeholder, Disk, Disk Key, Disk UUID, Disk Path, Capacity MiB, Raw, Disk Mode, Sharing mode, Thin, Eagerly Scrub, Split, Write Through, Level, Shares, Reservation, Limit, Controller, Label, SCSI Unit #, Unit #, Shared Bus, Path, Raw LUN ID, Raw Comp. Mode, Annotation, Application, LOCATION, Reboot, Datacenter, Cluster, Host, Folder, OS according to the configuration file, OS according to the VMware Tools, VM ID, VM UUID
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| VM | `vm` |
| Powerstate | `powerstate` |
| Template | `template` |
| Disk | `disk` |
| Disk Key | `diskKey` |
| Disk UUID | `diskUuid` |
| Disk Path | `path` |
| Capacity MiB | `capacityMiB` |
| Raw | `raw` |
| Disk Mode | `diskMode` |
| Thin | `thin` |
| Level | `level` |
| Shares | `shares` |
| Reservation | `reservation` |
| Limit | `limit` |
| Controller | `controller` |
| Unit # | `unitNumber` |
| Annotation | `annotation` |
| Host | `host` |


### vPartition

**RVTools columns:**

```
VM, Powerstate, Template, SRM Placeholder, Disk Key, Disk, Capacity MiB, Consumed MiB, Free MiB, Free %, Annotation, Application, LOCATION, Reboot, Datacenter, Cluster, Host, Folder, OS according to the configuration file, OS according to the VMware Tools, VM ID, VM UUID
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| VM | `vm` |
| Powerstate | `powerstate` |
| Template | `template` |
| Disk | `disk` |
| Capacity MiB | `capacityMiB` |
| Consumed MiB | `consumedMiB` |
| Free MiB | `freeMiB` |
| Free % | `freePercent` |
| Annotation | `annotation` |
| Host | `host` |


### vNetwork

**RVTools columns:**

```
VM, Powerstate, Template, SRM Placeholder, NIC label, Adapter, Network, Switch, Connected, Starts Connected, Mac Address, Type, IPv4 Address, IPv6 Address, Direct Path IO, Annotation, Application, LOCATION, Reboot, Datacenter, Cluster, Host, Folder, OS according to the configuration file, OS according to the VMware Tools, VM ID, VM UUID
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| VM | `vm` |
| Powerstate | `powerstate` |
| Template | `template` |
| NIC label | `nicLabel` |
| Adapter | `adapter` |
| Network | `network` |
| Connected | `connected` |
| Starts Connected | `startsConnected` |
| Mac Address | `macAddress` |
| Type | `type` |
| Direct Path IO | `directPathIO` |
| Annotation | `annotation` |
| Host | `host` |


### vCD

**RVTools columns:**

```
VM, Powerstate, Template, SRM Placeholder, Device Node, Connected, Starts Connected, Device Type, Annotation, Application, LOCATION, Reboot, Datacenter, Cluster, Host, Folder, OS according to the configuration file, OS according to the VMware Tools, VMRef, VM ID, VM UUID
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| VM | `vm` |
| Powerstate | `powerstate` |
| Template | `template` |
| Device Node | `deviceNode` |
| Connected | `connected` |
| Starts Connected | `startsConnected` |
| Device Type | `deviceType` |
| Annotation | `annotation` |
| Host | `host` |


### vUSB

**RVTools columns:**

```
VM, Powerstate, Template, SRM Placeholder, Device Node, Device Type, Connected, Family, Speed, EHCI enabled, Auto connect, Bus number, Unit number, Annotation, Application, LOCATION, Reboot, Datacenter, Cluster, Host, Folder, OS according to the configuration file, OS according to the VMware tools, VMRef, VM ID, VM UUID
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| VM | `vm` |
| Powerstate | `powerstate` |
| Template | `template` |
| Device Node | `deviceNode` |
| Device Type | `deviceType` |
| Connected | `connected` |
| Speed | `speed` |
| Annotation | `annotation` |
| Host | `host` |


### vSnapshot

**RVTools columns:**

```
VM, Powerstate, Name, Description, Date / time, Filename, Size MiB (vmsn), Size MiB (total), Quiesced, State, Annotation, Application, LOCATION, Reboot, Datacenter, Cluster, Host, Folder, OS according to the configuration file, OS according to the VMware Tools, VM ID, VM UUID
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| VM | `vm` |
| Powerstate | `powerstate` |
| Name | `name` |
| Description | `description` |
| Date / time | `dateTime` |
| Quiesced | `quiesced` |
| State | `state` |
| Annotation | `annotation` |
| Host | `host` |


### vTools

**RVTools columns:**

```
VM, Powerstate, Template, SRM Placeholder, VM Version, Tools, Tools Version, Required Version, Upgradeable, Upgrade Policy, Sync time, App status, Heartbeat status, Kernel Crash state, Operation Ready, State change support, Interactive Guest, Annotation, Application, LOCATION, Reboot, Datacenter, Cluster, Host, Folder, OS according to the configuration file, OS according to the VMware Tools, VMRef, VM ID, VM UUID
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| VM | `name` |
| Powerstate | `power_state` |
| Template | `template` |
| VM Version | `hwVersion` |
| Tools | `toolsVersionStatus` |
| Tools Version | `toolsVersionNumber` |
| Upgrade Policy | `toolsUpgradePolicy` |
| Sync time | `toolsSyncTime` |
| Heartbeat status | `guestHeartbeatStatus` |
| Annotation | `annotation` |
| Host | `host` |


### vSource

**RVTools columns:**

```
Name, OS type, API type, API version, Version, Patch level, Build, Fullname, Product name, Product version, Product line, Vendor
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| Name | `name` |
| OS type | `osType` |
| API type | `apiType` |
| API version | `apiVersion` |
| Version | `version` |
| Patch level | `patchLevel` |
| Build | `build` |
| Fullname | `fullname` |
| Product name | `productName` |
| Product version | `productVersion` |
| Product line | `productLine` |
| Vendor | `vendor` |


### vRP

**RVTools columns:**

```
Resource Pool name, Resource Pool path, Status, # VMs total, # VMs, # vCPUs, CPU limit, CPU overheadLimit, CPU reservation, CPU level, CPU shares, CPU expandableReservation, CPU maxUsage, CPU overallUsage, CPU reservationUsed, CPU reservationUsedForVm, CPU unreservedForPool, CPU unreservedForVm, Mem Configured, Mem limit, Mem overheadLimit, Mem reservation, Mem level, Mem shares, Mem expandableReservation, Mem maxUsage, Mem overallUsage, Mem reservationUsed, Mem reservationUsedForVm, Mem unreservedForPool, Mem unreservedForVm, QS overallCpuDemand, QS overallCpuUsage, QS staticCpuEntitlement, QS distributedCpuEntitlement, QS balloonedMemory, QS compressedMemory, QS consumedOverheadMemory, QS distributedMemoryEntitlement, QS guestMemoryUsage, QS hostMemoryUsage, QS overheadMemory, QS privateMemory, QS sharedMemory, QS staticMemoryEntitlement, QS swappedMemory
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| Resource Pool name | `resourcePoolName` |
| Status | `status` |
| CPU limit | `cpuLimit` |
| CPU reservation | `cpuReservation` |
| CPU level | `cpuLevel` |
| CPU shares | `cpuShares` |
| Mem limit | `memLimit` |
| Mem reservation | `memReservation` |
| Mem level | `memLevel` |
| Mem shares | `memShares` |
| QS overallCpuUsage | `qsOverallCpuUsage` |
| QS guestMemoryUsage | `qsGuestMemoryUsage` |


### vCluster

**RVTools columns:**

```
Name, Config status, OverallStatus, NumHosts, numEffectiveHosts, TotalCpu, NumCpuCores, NumCpuThreads, Effective Cpu, TotalMemory, Effective Memory, Num VMotions, HA enabled, Failover Level, AdmissionControlEnabled, Host monitoring, HB Datastore Candidate Policy, Isolation Response, Restart Priority, Cluster Settings, Max Failures, Max Failure Window, Failure Interval, Min Up Time, VM Monitoring, DRS enabled, DRS default VM behavior, DRS vmotion rate, DPM enabled, DPM default behavior, DPM Host Power Action Rate, com.vmware.vcenter.cluster.edrs.upgradeHostAdded
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| Name | `clusterName` |
| DRS enabled | `drsEnabled` |


### vHost

**RVTools columns:**

```
Host, Datacenter, Cluster, Config status, Compliance Check State, in Maintenance Mode, in Quarantine Mode, vSAN Fault Domain Name, CPU Model, Speed, HT Available, HT Active, # CPU, Cores per CPU, # Cores, CPU usage %, # Memory, Memory Tiering Type, Memory usage %, Console, # NICs, # HBAs, # VMs total, # VMs, VMs per Core, # vCPUs, vCPUs per Core, vRAM, VM Used memory, VM Memory Swapped, VM Memory Ballooned, VMotion support, Storage VMotion support, Current EVC, Max EVC, Assigned License(s), ATS Heartbeat, ATS Locking, Current CPU power man. policy, Supported CPU power man., Host Power Policy, ESX Version, Boot time, DNS Servers, DHCP, Domain, Domain List, DNS Search Order, NTP Server(s), NTPD running, Time Zone, Time Zone Name, GMT Offset, Vendor, Model, Serial number, Service tag, OEM specific string, BIOS Vendor, BIOS Version, BIOS Date, Certificate Issuer, Certificate Start Date, Certificate Expiry Date, Certificate Status, Certificate Subject, AutoDeploy.MachineIdentity, UUID, Host, LOCATION
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| Host | `name` |
| Connection State | `connectionState` |
| Power State | `powerState` |
| # Cores | `cpuCores` |
| # CPU | `cpuSockets` |
| CPU Threads | `cpuThreads` |
| CPU usage % | `cpuUsagePercent` |
| Memory usage % | `memoryUsagePercent` |
| Vendor | `vendor` |
| Model | `model` |
| CPU Model | `cpuModel` |
| # NICs | `numNics` |
| # HBAs | `numHbas` |
| HT Available | `htAvailable` |
| HT Active | `htActive` |
| ESX Version | `esxVersion` |
| Boot time | `bootTime` |
| Current EVC | `currentEvcMode` |
| Max EVC | `maxEvcMode` |
| # Memory (GiB) | `memoryGiB` |
| Speed (MHz) | `cpuSpeedMhz` |
| VMotion support | `vmotionEnabled` |
| Domain | `domain` |
| Time Zone | `timeZone` |
| BIOS Version | `biosVersion` |
| BIOS Date | `biosDate` |
| Config status | `configStatus` |
| in Maintenance Mode | `inMaintenanceMode` |
| Current CPU power man. policy | `cpuPowerPolicy` |
| UUID | `uuid` |


### vHBA

**RVTools columns:**

```
Host, Datacenter, Cluster, Device, Type, Status, Bus, Pci, Driver, Model, WWN
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| Host | `host` |
| Device | `device` |
| Type | `type` |
| Status | `status` |
| Bus | `bus` |
| Pci | `pci` |
| Driver | `driver` |
| Model | `model` |
| WWN | `wwn` |


### vNIC

**RVTools columns:**

```
Host, Datacenter, Cluster, Network Device, Driver, Speed, Duplex, MAC, Switch, Uplink port, PCI, WakeOn
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| Host | `host` |
| Network Device | `networkDevice` |
| Driver | `driver` |
| Speed | `speed` |
| Duplex | `duplex` |
| MAC | `mac` |
| PCI | `pci` |


### vSwitch

**RVTools columns:**

```
Host, Datacenter, Cluster, Switch, # Ports, Free Ports, Promiscuous Mode, Mac Changes, Forged Transmits, Traffic Shaping, Width, Peak, Burst, Policy, Reverse Policy, Notify Switch, Rolling Order, Offload, TSO, Zero Copy Xmit, MTU
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| Host | `host` |
| Switch | `switch` |
| # Ports | `numPorts` |
| Free Ports | `freePorts` |
| Promiscuous Mode | `promiscuousMode` |
| Mac Changes | `macChanges` |
| Forged Transmits | `forgedTransmits` |
| MTU | `mtu` |


### vPort

**RVTools columns:**

```
Host, Datacenter, Cluster, Port Group, Switch, VLAN, Promiscuous Mode, Mac Changes, Forged Transmits, Traffic Shaping, Width, Peak, Burst, Policy, Reverse Policy, Notify Switch, Rolling Order, Offload, TSO, Zero Copy Xmit
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| Host | `host` |
| Port Group | `portGroup` |
| Switch | `switch` |
| VLAN | `vlan` |
| Promiscuous Mode | `promiscuousMode` |
| Mac Changes | `macChanges` |
| Forged Transmits | `forgedTransmits` |


### dvSwitch

**RVTools columns:**

```
Switch, Datacenter, Name, Vendor, Version, Description, Created, Host members, Max Ports, # Ports, # VMs, In Traffic Shaping, In Avg, In Peak, In Burst, Out Traffic Shaping, Out Avg, Out Peak, Out Burst, CDP Type, CDP Operation, LACP Name, LACP Mode, LACP Load Balance Alg., Max MTU, Contact, Admin Name
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| Switch | `switch` |
| Vendor | `vendor` |
| Version | `version` |
| Created | `created` |
| Host members | `hostMembers` |
| Max Ports | `maxPorts` |
| # Ports | `numPorts` |


### dvPort

**RVTools columns:**

```
Port, Switch, Type, # Ports, VLAN, Speed, Full Duplex, Blocked, Allow Promiscuous, Mac Changes, Active Uplink, Standby Uplink, Policy, Forged Transmits, In Traffic Shaping, In Avg, In Peak, In Burst, Out Traffic Shaping, Out Avg, Out Peak, Out Burst, Reverse Policy, Notify Switch, Rolling Order, Check Beacon, Live Port Moving, Check Duplex, Check Error %, Check Speed, Percentage, Block Override, Config Reset, Shaping Override, Vendor Config Override, Sec. Policy Override, Teaming Override, Vlan Override
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| Port | `port` |
| Switch | `switch` |
| VLAN | `vlan` |
| Promiscuous Mode | `promiscuousMode` |
| Mac Changes | `macChanges` |
| Forged Transmits | `forgedTransmits` |
| Policy | `policy` |
| Reverse Policy | `reversePolicy` |
| Notify Switch | `notifySwitch` |
| Rolling Order | `rollingOrder` |
| In Traffic Shaping | `inTrafficShaping` |
| Out Traffic Shaping | `outTrafficShaping` |


### vSC_VMK

**RVTools columns:**

```
Host, Datacenter, Cluster, Port Group, Device, Mac Address, DHCP, IP Address, IP 6 Address, Subnet mask, Gateway, IP 6 Gateway, MTU
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| Host | `host` |
| Port Group | `portGroup` |
| Device | `device` |
| Mac Address | `macAddress` |
| DHCP | `dhcp` |
| IP Address | `ipAddress` |
| Subnet mask | `subnetMask` |
| Gateway | `gateway` |
| MTU | `mtu` |


### vDatastore

**RVTools columns:**

```
Name, Config status, Address, Accessible, Type, # VMs total, # VMs, Capacity MiB, Provisioned MiB, In Use MiB, Free MiB, Free %, SIOC enabled, SIOC Threshold, # Hosts, Hosts, Cluster name, Cluster capacity MiB, Cluster free space MiB, Block size, Max Blocks, # Extents, Major Version, Version, VMFS Upgradeable, MHA, URL
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| Name | `name` |
| Type | `type` |
| Capacity GiB | `capacityGiB` |
| Free GiB | `freeGiB` |
| Free % | `freePercent` |


### vMultiPath

> **Not implemented.** Needs HostMultipathInfo / storage path enumeration

**RVTools columns:**

```
Host, Cluster, Datacenter, Datastore, Disk, Display name, Policy, Oper. State, Path 1, Path 1 state, Path 2, Path 2 state, Path 3, Path 3 state, Path 4, Path 4 state, Path 5, Path 5 state, Path 6, Path 6 state, Path 7, Path 7 state, Path 8, Path 8 state, vStorage, Queue depth, Vendor, Model, Revision, Level, Serial #, UUID
```



### vLicense

**RVTools columns:**

```
Name, Key, Labels, Cost Unit, Total, Used, Expiration Date, Features
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| Name | `name` |
| Key | `key` |
| Cost Unit | `costUnit` |
| Total | `total` |
| Expiration Date | `expirationDate` |


### vFileInfo

> **Not implemented.** Needs HostDatastoreBrowser + SearchDatastoreSubFolders (datastore file-tree walking)

**RVTools columns:**

```
Friendly Path Name, File Name, File Type, File Size in bytes, Path
```



### vHealth

> **Correction (verified 2026-08-31).** The note that this needs
> EventManager / AlarmManager aggregation is **wrong**. Every row in the
> reference export is a check RVTools computes itself from inventory it has
> already collected — there is not a single vCenter alarm in the sheet. The 43
> rows break down as: Foldername 10, CDROM 10, Zombie 6, NTP 5, NTPD 5,
> Snapshot 5, Performance tip 2.

**RVTools columns:**

```
Name, Message, Message type
```

**Checks and their exact wording**, read out of the reference export:

| Type | Message | Source | Implemented |
|---|---|---|---|
| `NTP` | `NTP Server value is null!` | `config.dateTimeInfo.ntpConfig.server` empty | yes |
| `NTPD` | `NTPD service is not running!` | `config.service.service` key `ntpd` | yes |
| `Foldername` | `Inconsistent Foldername! VMname = {vm} Foldername = {folder}` | folder in `config.files.vmPathName` vs VM name, **case-sensitive** | yes |
| `CDROM` | `VM has a CDROM device connected! {label}` | `VirtualCdrom` with `connectable.connected` | yes |
| `Snapshot` | `VM has an active snapshot! {name} created on {yyyy/MM/dd HH:mm:ss}` | `snapshot.rootSnapshotList`, flattened | yes |
| `Zombie` | `Possibly a Zombie vmdk file! Please check.` | datastore file walk vs registered disks | **no** — needs `HostDatastoreBrowser` / `SearchDatastoreSubFolders` |
| `Performance tip` | `In-Memory VM performance improvement possible! Please check documentation` | unknown | **no** — trigger not determinable from the export |

RVTools emits host findings before VM findings, and groups a VM's findings
together; we do the same.



### vMetaData

**RVTools columns:**

```
RVTools major version, RVTools version, xlsx creation datetime, Server
```

**Implemented** (RVTools label → JSON key):

| Column | Key |
|---|---|
| App Name | `appName` |
| App Version | `appVersion` |
| Export DateTime | `exportDateTime` |
| Server | `server` |

