# CLAUDE.md - STTools

Drop this at the root of the new repo. Claude Code reads it automatically at the
start of every session, so it front-loads the domain knowledge that would
otherwise cost debugging cycles to rediscover.

## Repo layout — read this first

This repo is **STTools**, a fork of
`dalehassinger/VMware-Explore-Hackathon-2026-Live`. All work happens here and is
pushed to `origin` (`virtualFrog/STTools`). The original repo is wired up as
`upstream` and is read-only for us; pull from it with `git fetch upstream`.

A prior working implementation exists at `../VMware-Explore-Hackathon-2026/Tauri/`
in the original author's setup. It is **not checked out in this working copy**, so
treat any reference to it below as conditional on cloning it yourself. Where it is
available, consult it for vCenter property paths, SOAP request shapes and RVTools
column mappings.

- It is **reference only. Never modify anything under it.**
- Treat it as a source of understanding, not code to copy. Write fresh
  implementations here.
- It has known defects; the "Improvements required" list in `docs/BUILD-PLAN.md`
  says what to do differently. Don't reproduce them.

## Companion references in this repo

These live under `docs/`.

- `docs/LAB-ENVIRONMENT.md` — vCenter host, credentials, what's in the lab, and which
  empty results are expected rather than bugs
- `docs/VCENTER-PROPERTY-REFERENCE.md` — 89 verified vim25 property paths by object type
- `docs/RVTOOLS-SHEETS-AND-COLUMNS.md` — all 27 RVTools sheets with exact column names,
  and how much of each the reference implementation covers
- `docs/RUNNING-ON-WINDOWS.md` — prerequisites, commands and troubleshooting for
  building and running on Windows

---

## What we're building

A cross-platform desktop app (and optional web service) that pulls VMware vCenter
inventory and presents it as sortable tables, one per object type — a native
alternative to RVTools, which is Windows-only. Data can be exported to a
multi-sheet `.xlsx` workbook matching RVTools' sheet and column naming.

**Stack:** Tauri v2 (Rust backend, plain HTML/CSS/vanilla JS frontend — no framework, no build step).

## Non-negotiable ground rules

1. **Never guess a vCenter API field name or XML shape.** Query the live vCenter
   with `curl` first, look at the actual response, then write the parsing code.
   Guessing has produced silent empty tables more than once.
2. **Verify against the live system before declaring something done.** Compare
   row counts to what vCenter itself reports.
3. Match RVTools' exact sheet and column names where an equivalent exists. If our
   units differ (GiB vs MiB), keep RVTools' term but state the real unit — never
   label GiB values as MiB.

---

## vCenter API essentials

vCenter exposes **two** APIs and you need both.

### REST — easy, but limited coverage

- Login: `POST /rest/com/vmware/cis/session` with HTTP basic auth → returns
  `{"value": "<token>"}`. Send it back as the `vmware-api-session-id` header.
- Two namespaces exist and differ: legacy `/rest/vcenter/*` and newer
  `/api/vcenter/*`. Some endpoints only exist in one. **Sessions are shared
  between them** — one token works for both.
- Good for: host list, VM list, clusters, datastores, resource pools, networks.
- Missing: nearly all hardware detail, per-VM devices, performance stats.

### SOAP (vim25) — everything else

- Endpoint: `POST /sdk`, `Content-Type: text/xml; charset=utf-8`,
  `SOAPAction: urn:vim25/8.0`.
- Login: `Login` on `SessionManager` → grab the `vmware_soap_session` cookie from
  `Set-Cookie` and send it on subsequent calls.
- Read properties via `RetrievePropertiesEx` on `propertyCollector`.
- `RetrieveServiceContent` on `ServiceInstance` needs **no** auth — handy for
  version info.

### The single biggest gotcha

**In vim25 SOAP responses, array elements are named after the property's declared
field _type_, not the field name.**

```
config.hardware.device  (VirtualDevice[])  →  <VirtualDevice xsi:type="VirtualDisk">   NOT <device>
guest.disk              (GuestDiskInfo[])  →  <GuestDiskInfo>                          NOT <disk>
snapshot.rootSnapshotList                  →  <VirtualMachineSnapshotTree>
```

Getting this wrong yields **zero rows with no error**. If a new SOAP array query
returns nothing, check the element names first — dump the raw XML and look.

**This applies only to the top-level `<val>` array.** Arrays *nested inside* a
data object repeat the **field** name instead. Verified 2026-08-31 against the
lab:

```
config.hardware.device      → <VirtualDevice xsi:type="VirtualDisk">   (type name)
  …its storageIOAllocation  → <shares><shares>1000</shares>…           (field name)
layoutEx.snapshot           → <VirtualMachineFileLayoutExSnapshotLayout>  (type name)
  …its disk chain           → <disk><chain><fileKey>3</fileKey>…       (field name)
snapshot.rootSnapshotList   → <VirtualMachineSnapshotTree>             (type name)
  …its children             → <childSnapshotList>                      (field name)
```

### Other API facts worth knowing

- Lab vCenters use self-signed certs → the HTTP client needs
  `danger_accept_invalid_certs(true)`.
- **Escape XML when interpolating credentials into SOAP envelopes.** A password
  containing `&`, `<`, or `>` produces malformed XML and a confusing failure
  where REST-backed views work and SOAP-backed ones don't.
- Snapshots nest (a snapshot can have children) — flatten recursively.
- Not everything is available: `vMultiPath` and `vFileInfo` need datastore file
  browsing, a different API area entirely. Treat as out of scope unless you have
  time to spare.
- `vHealth` was previously listed here too, on the belief that it needs
  alarm/event aggregation. **It does not** — RVTools computes it from inventory
  it already has (NTP, NTPD, folder-name, CDROM and snapshot checks), so it is
  cheap. Only its `Zombie` check needs datastore browsing. See
  `docs/RVTOOLS-SHEETS-AND-COLUMNS.md`.

---

## Session management (get this right from day one)

vCenter sessions **do not** clean themselves up promptly — they linger until a
~30 minute idle timeout. Logging in per API call leaks sessions fast; an earlier
version of this app accumulated ~300 open sessions in a day of testing.

- Cache and reuse sessions. Key the cache by **host + username** so multiple
  vCenters don't evict each other.
- Refresh on a TTL (15 min is safe against the 30 min idle timeout).
- Log out explicitly: REST `DELETE /rest/com/vmware/cis/session`, SOAP `Logout`
  on `SessionManager`.
- Clean up on shutdown, handling **both SIGINT and SIGTERM** — `systemctl
  stop/restart` sends SIGTERM, so Ctrl-C-only handling leaks on every restart.

Check open sessions with `RetrievePropertiesEx` on `SessionManager` /
`sessionList` and count `<UserSession` elements.

---

## Architecture that worked

Learned from the reference implementation — worth adopting from the start rather
than refactoring into later.

### Separate core logic from the UI framework

Each data source is a plain function:

```rust
pub async fn fetch_host_data_core(conn: &VCenterConnection) -> Result<Vec<HostInfo>, String>
```

Tauri commands are thin wrappers around these. This is what makes a web-server
binary possible later without touching any query logic — retrofitting it meant
mechanically rewriting 21 functions.

### Design for multiple vCenters from the start

Config should hold a **list** of connections, not one. Wrap each per-server
function with an aggregator that loops all servers, concatenates rows, and tags
each row with its source vCenter (RVTools calls this column `VI SDK Server`).
Bolting this on afterwards touched the config shape, every command, the session
cache, the settings UI, and the export.

When one vCenter is unreachable, return data from the healthy ones plus a visible
warning. Never fail everything, and never silently under-report — this is an
inventory tool, so a short list that looks complete is the worst outcome.

### Don't duplicate column definitions

Define each table's columns once. When adding a cross-cutting column (like
`VI SDK Server`), append it **generically** in the table renderer and the xlsx
writer — two places, not once per table. There are ~24 tables; editing them all
by hand is where mistakes creep in.

### Surface errors, don't swallow them

Avoid `let Ok(x) = ... else { continue }` in per-object loops. A host that fails
to query should be reported, not silently dropped from the results.

---

## Security (build it right the first time)

The reference implementation got these wrong; don't inherit them.

- **Escape HTML when rendering vCenter data.** VM annotations and names are
  free-text and become XSS if interpolated into `innerHTML`. In a Tauri webview
  with `withGlobalTauri: true`, injected script can reach `window.__TAURI__` and
  call backend commands.
- **If you build a web server, authenticate it.** Binding `0.0.0.0` with no auth
  and an endpoint that returns stored vCenter credentials in cleartext exposes
  admin passwords to the whole network.
- Don't log credentials.

---

## Build/run notes

- `npm run tauri dev` — run the desktop app.
- `npm run tauri build` — produce an installer. It only builds for the platform
  you're on; Windows/Linux installers need to be built on those platforms.
  Windows setup is written up in `docs/RUNNING-ON-WINDOWS.md`.
- Adding a second binary (e.g. a web server) breaks `cargo run`, which
  `tauri dev` uses internally. Fix with `default-run = "<app-name>"` in
  `Cargo.toml`'s `[package]`.
- Keep tests free of absolute machine-specific paths — write to `std::env::temp_dir()`.
