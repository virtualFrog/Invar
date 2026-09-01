# Hackathon Build Plan — Reference-While-Rewriting

Build the new app in a fresh repo, with Claude Code reading the existing
implementation as a reference and writing new code from that understanding.

The goal is **not** transcription. The reference tells you *what to build and
which vCenter properties to use*; the new repo should be a cleaner
implementation that fixes the reference's known defects.

---

## Setup

### 1. Put both repos side by side

```
~/Documents/GitHub/
├── VMware-Explore-Hackathon-2026/        ← REFERENCE (read-only)
└── VMware-Explore-Hackathon-2026-Live/   ← NEW repo (all work happens here)
```

⚠️ The names differ only by the `-Live` suffix. Before any session, confirm you
are in the right one — writing into the reference repo by mistake is easy and
annoying to unpick:

```bash
pwd   # must end in -Live
```

### 2. Start Claude Code from the new repo, with read access to the reference

```bash
cd ~/Documents/GitHub/VMware-Explore-Hackathon-2026-Live
claude --add-dir ~/Documents/GitHub/VMware-Explore-Hackathon-2026
```

Or `/add-dir <path>` mid-session. Confirm both are wired up correctly before
relying on it:

```
Confirm two things:
1. My working directory — it should end in -Live and be nearly empty.
2. That you can read ../VMware-Explore-Hackathon-2026/Tauri/src-tauri/src/lib.rs
   — list its first few function names.

All files you create go in the -Live repo. Never modify anything under
VMware-Explore-Hackathon-2026 (no -Live suffix); it is reference only.
```

### 3. Seed the new repo

Copy from this kit into the new repo root:

- `CLAUDE.md` — read automatically each session
- `VCENTER-PROPERTY-REFERENCE.md` — the 89 verified property paths

Then append this to `CLAUDE.md` so every session knows the reference exists:

```markdown
## Reference implementation

A prior working implementation lives at
`../VMware-Explore-Hackathon-2026/Tauri/`. Consult it for vCenter property
paths, SOAP request shapes, and RVTools column mappings.

Treat it as a reference, not a source to copy from. Write fresh implementations
here. It has known defects listed under "Improvements required" below — do not
reproduce them.
```

---

## Improvements required over the reference

Bake these in from the start. Retrofitting each of them into the reference was
painful, and they make a good "here's how ours is better" line for judging.

| # | Reference defect | Do instead |
|---|---|---|
| 1 | Query logic entangled with Tauri types; had to be mechanically split across 21 functions to add a web server | Keep core query functions free of UI-framework types from day one |
| 2 | Single-vCenter config; multi-vCenter retrofit touched config, every command, session cache, settings UI and export | Config holds a **list** of connections from the start |
| 3 | Logged in per API call, never logged out — leaked ~300 vCenter sessions | Cache sessions keyed by host+username, TTL'd, with explicit logout on shutdown (SIGINT **and** SIGTERM) |
| 4 | Same per-VM device array queried 4× (vDisk/vNetwork/vCD/vUSB); host network array 5× | Query once, split into the sheets that need it |
| 5 | vCenter strings interpolated into `innerHTML` → XSS via VM annotations | Escape all vCenter-supplied text before rendering |
| 6 | Credentials interpolated into SOAP XML unescaped → login silently breaks on passwords containing `&`, `<`, `>` | XML-escape credentials |
| 7 | Web server had no authentication and served stored vCenter passwords in cleartext | Authenticate anything bound beyond localhost; never return stored passwords |
| 8 | Failed per-object queries silently dropped rows | Surface failures; never silently under-report |
| 9 | Test wrote to a hardcoded absolute path | Use `std::env::temp_dir()` |

---

## Phase 1 — One vertical slice (captain, solo)

Establishes the pattern everyone else copies. Don't parallelize before this exists.

```
Read ../VMware-Explore-Hackathon-2026/Tauri/src-tauri/src/lib.rs and
../VMware-Explore-Hackathon-2026/Tauri/src/main.js to understand how the
reference implementation connects to vCenter and renders a table.

Before writing any code, summarize:
- how it authenticates to the REST API
- how its data flows from Rust to the frontend
- what you'd do differently given the "Improvements required" list in CLAUDE.md

Then build a first vertical slice in THIS repo — fresh code, not copied:

1. Tauri v2 app, Rust backend, plain HTML/CSS/vanilla JS (no framework, no build step).
2. Settings storing a LIST of vCenter connections (host/username/password),
   persisted as JSON in the OS app-config directory. Multi-vCenter from the start.
3. `fetch_host_data_core(conn: &VCenterConnection) -> Result<Vec<HostInfo>, String>`
   returning ESXi hosts (name, connection state, power state) from the REST API.
   No Tauri types in this function.
4. A "vHost" tab rendering a sortable table, with all vCenter text HTML-escaped.

Verify against my live vCenter (<HOST>, user <USER>) and confirm the row count
matches what vCenter reports.
```

**Checkpoint:** real hosts on screen, count matches vCenter. Commit.

---

## Phase 2 — Shared plumbing (captain, solo)

```
Read how the reference implements SOAP querying and session handling in
../VMware-Explore-Hackathon-2026/Tauri/src-tauri/src/lib.rs — specifically
soap_request, parse_prop_set, parse_xml_tree, find_prop_val, and the session
cache.

Explain how its XML parsing handles the difference between scalar properties and
array-valued ones, then implement equivalents here in your own way.

Requirements:
- Session cache keyed by host+username (the reference originally used a single
  slot, which made multiple vCenters evict each other's sessions).
- Explicit REST and SOAP logout, called on shutdown for both SIGINT and SIGTERM.
- XML-escape credentials in the SOAP Login envelope.
- One query per object type that serves all sheets needing it — do not query the
  same device array once per sheet.

Then extend vHost with SOAP data: CPU cores/sockets/threads, CPU and memory
usage %, vendor, model, ESXi version. Use the paths in
VCENTER-PROPERTY-REFERENCE.md, but curl-verify the response shape first.
```

**Checkpoint:** vHost fully populated; session count on vCenter stays flat across
repeated refreshes. Commit and push — the team branches from here.

---

## Phase 3 — Parallelize (one sheet per person)

Each sheet is an independent `fetch_x_core` plus a frontend column list.

```
Add a "<SHEET>" tab.

Reference: see how ../VMware-Explore-Hackathon-2026/Tauri/src-tauri/src/lib.rs
sources this data, and check VCENTER-PROPERTY-REFERENCE.md for the property
paths. Follow the conventions already established by vHost in this repo.

RVTools column headers for this sheet: <PASTE HEADERS>

1. curl the relevant API and show me the actual response before parsing.
2. Reuse an existing query if one already fetches this object — don't add a
   redundant round trip.
3. Add the tab and column list; escape all rendered values.
4. Tell me which RVTools columns you could not source, and why.
```

**Merge hotspots:** command registration, tab bar markup, export sheet list.
Either add stubs for all planned sheets upfront so people edit distinct regions,
or have one person own merges.

---

## Phase 4 — Export (one person, once ~3 sheets exist)

```
Add XLSX export: one sheet per tab in a single workbook.

The reference does this in build_workbook_bytes / xlsx_add_sheet — read it for
the approach and for the RVTools formatting it matched (inspect the reference
xlsx at <PATH> yourself to confirm header font, fill, freeze panes, autofilter).

Requirements:
- Workbook building returns bytes, separate from file saving, so other frontends
  can reuse it.
- Numeric-looking strings become real numbers, but UUIDs, MAC addresses and
  version strings stay text.
- Add the source-vCenter column automatically in ONE place, not per sheet.
```

---

## Phase 5 — Demo differentiators

Pick by remaining time:

- **Left sidebar UI** grouped by category — scales past ~10 tabs
- **Performance gauges** — most visually striking view
- **Web server binary** reusing the same core functions and frontend (frontend
  needs only a shim swapping Tauri `invoke` for `fetch('/api/...')`). Set
  `default-run` in `Cargo.toml`, and **add authentication**.
- **Something the reference doesn't do** — drift comparison between two vCenters,
  or change tracking between exports. Strongest judging angle.

---

## Making "reference while rewriting" actually work

- **Always ask for a summary before implementation.** "Explain how the reference
  does X, then implement it here" produces understanding; "port X" produces
  transcription.
- **Review the diff.** If output is line-for-line identical to the reference,
  push back and ask for the approach in this repo's own conventions.
- **Name the improvement.** Every phase above cites the reference defect it fixes
  — that's what makes this a rewrite rather than a copy.
- **Don't reference the UI code much.** It's the least interesting part and the
  most tempting to paste. Better to describe the UI you want and let it be built
  fresh.

## Demo-day checklist

- [ ] Runs from a clean clone — no absolute paths from a dev machine
- [ ] Settings works; judges may point it at their own vCenter
- [ ] Export opens cleanly in Excel
- [ ] Session count on vCenter stays flat after repeated use
- [ ] Fallback screenshots/recording in case event Wi-Fi can't reach your lab
- [ ] Know your gaps — "vMultiPath needs datastore file browsing, so we scoped it
      out" beats being surprised
