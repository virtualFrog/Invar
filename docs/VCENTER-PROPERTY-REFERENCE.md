# vCenter Property Reference

Extracted from a working implementation and **verified against a live vCenter
9.1**. This is the expensive part of the project — each of these took curl
round-trips to discover. Use it so you don't repeat that.

Everything below is a *starting point*, not gospel: property availability varies
by vCenter version and licensing. Still curl-verify before parsing, but this
tells you where to look.

## How to query these

REST: `POST /rest/com/vmware/cis/session` with basic auth, then send the returned
token as the `vmware-api-session-id` header.

SOAP: `POST /sdk` with `SOAPAction: urn:vim25/8.0`, `Login` on `SessionManager`
to get a `vmware_soap_session` cookie, then `RetrievePropertiesEx` on
`propertyCollector`:

```xml
<vim25:RetrievePropertiesEx>
  <vim25:_this type="PropertyCollector">propertyCollector</vim25:_this>
  <vim25:specSet>
    <vim25:propSet>
      <vim25:type>HostSystem</vim25:type>
      <vim25:pathSet>hardware.cpuInfo.numCpuCores</vim25:pathSet>
    </vim25:propSet>
    <vim25:objectSet>
      <vim25:obj type="HostSystem">host-9</vim25:obj>
    </vim25:objectSet>
  </vim25:specSet>
  <vim25:options/>
</vim25:RetrievePropertiesEx>
```

**Remember:** array-valued properties serialize elements named after the declared
field *type*, not the field name — `config.hardware.device` yields
`<VirtualDevice xsi:type="VirtualDisk">`, `guest.disk` yields `<GuestDiskInfo>`,
`snapshot.rootSnapshotList` yields `<VirtualMachineSnapshotTree>`. Get this wrong
and you get zero rows with no error.

---

## REST endpoints used

These cover object listing; almost everything else needs SOAP.


- `/api/vcenter/datastore`
- `/api/vcenter/network`
- `/api/vcenter/resource-pool`
- `/rest/com/vmware/cis/session`
- `/rest/vcenter/cluster`
- `/rest/vcenter/host`
- `/rest/vcenter/host/`
- `/rest/vcenter/vm`
- `/rest/vcenter/vm/`

Note: `/rest/vcenter/*` and `/api/vcenter/*` are different namespaces with
different coverage, but share session tokens. Per-host and per-VM *detail*
endpoints largely do not exist — that's why SOAP is unavoidable.

---

## SOAP (vim25) properties by object type


### DistributedVirtualPortgroup

Backs dvPort. `config.defaultPortConfig` contains VLAN plus security/teaming/shaping policy, each wrapped in `<value>` elements.

```
config.defaultPortConfig
config.distributedVirtualSwitch
```

### HostSystem

Backs the vHost sheet. The `config.network.*` and `config.storageDevice.hostBusAdapter` arrays back vNIC/vSwitch/vPort/vSC_VMK/vHBA — one query can serve all five.

```
config.dateTimeInfo.timeZone.name
config.hyperThread.active
config.hyperThread.available
config.network.dnsConfig.domainName
config.network.pnic
config.network.portgroup
config.network.vnic
config.network.vswitch
config.storageDevice.hostBusAdapter
hardware.biosInfo.biosVersion
hardware.biosInfo.releaseDate
hardware.cpuInfo.numCpuCores
hardware.cpuInfo.numCpuPackages
hardware.cpuInfo.numCpuThreads
hardware.cpuPowerManagementInfo.currentPolicy
hardware.systemInfo.uuid
overallStatus
runtime.bootTime
runtime.inMaintenanceMode
summary.config.product.fullName
summary.config.vmotionEnabled
summary.currentEVCModeKey
summary.hardware.cpuMhz
summary.hardware.cpuModel
summary.hardware.memorySize
summary.hardware.model
summary.hardware.numHBAs
summary.hardware.numNics
summary.hardware.vendor
summary.maxEVCModeKey
summary.quickStats.overallCpuUsage
summary.quickStats.overallMemoryUsage
```

### LicenseManager

Backs vLicense. Query object id `LicenseManager`.

```
licenses
```

### ResourcePool

Backs vRP.

```
config.cpuAllocation.limit
config.cpuAllocation.reservation
config.cpuAllocation.shares.level
config.cpuAllocation.shares.shares
config.memoryAllocation.limit
config.memoryAllocation.reservation
config.memoryAllocation.shares.level
config.memoryAllocation.shares.shares
overallStatus
summary.quickStats.guestMemoryUsage
summary.quickStats.overallCpuUsage
```

### VirtualMachine

Backs vInfo, vCPU, vMemory, vTools. `config.hardware.device` backs vDisk/vNetwork/vCD/vUSB — one query serves all four, so fetch it once and split.

```
config.annotation
config.bootOptions.efiSecureBootEnabled
config.changeVersion
config.cpuAllocation.limit
config.cpuAllocation.reservation
config.cpuAllocation.shares.level
config.cpuAllocation.shares.shares
config.cpuHotAddEnabled
config.cpuHotRemoveEnabled
config.createDate
config.files.vmPathName
config.firmware
config.hardware.device
config.hardware.numCoresPerSocket
config.memoryAllocation.limit
config.memoryAllocation.reservation
config.memoryAllocation.shares.level
config.memoryAllocation.shares.shares
config.memoryHotAddEnabled
config.memoryReservationLockedToMax
config.template
config.tools.syncTimeWithHost
config.tools.toolsUpgradePolicy
config.uuid
config.version
guest.disk
guest.toolsRunningStatus
guest.toolsVersion
guest.toolsVersionStatus
guestHeartbeatStatus
runtime.host
runtime.memoryOverhead
snapshot.rootSnapshotList
summary.config.memorySizeMB
summary.quickStats.balloonedMemory
summary.quickStats.consumedOverheadMemory
summary.quickStats.distributedCpuEntitlement
summary.quickStats.distributedMemoryEntitlement
summary.quickStats.guestMemoryUsage
summary.quickStats.hostMemoryUsage
summary.quickStats.overallCpuUsage
summary.quickStats.privateMemory
summary.quickStats.sharedMemory
summary.quickStats.staticCpuEntitlement
summary.quickStats.staticMemoryEntitlement
summary.quickStats.swappedMemory
summary.runtime.maxCpuUsage
summary.storage.committed
summary.storage.uncommitted
```

### VmwareDistributedVirtualSwitch

Backs dvSwitch. Reach it via a portgroup's `config.distributedVirtualSwitch` reference.

```
config.createTime
config.maxPorts
name
summary.hostMember
summary.numPorts
summary.productInfo
```

### Also useful

- `RetrieveServiceContent` on `ServiceInstance` — **needs no authentication**;
  returns vCenter name/version/build/API version (backs vSource).
- `SessionManager` / `sessionList` — count open sessions; invaluable for
  confirming you aren't leaking them.

---

## Known-unavailable

Don't burn hackathon time on these — they need different API areas entirely:

| RVTools sheet | Why it's hard |
|---|---|
| vMultiPath | Needs `HostMultipathInfo` / storage path enumeration |
| vFileInfo | Needs `HostDatastoreBrowser` + `SearchDatastoreSubFolders` (datastore file-tree walking) |
| vHealth | Needs `EventManager` / `AlarmManager` aggregation |

Individual columns also skipped: Cluster and Datacenter names on host/VM rows
(need MoRef→name traversal up the inventory tree), DNS/NTP server lists and host
serial numbers (array-typed properties), and snapshot file sizes (need `layoutEx`
cross-referencing).

