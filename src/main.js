// STTools frontend.
//
// Every value rendered here comes from vCenter and is free text (VM names,
// annotations). Nothing is ever written with innerHTML: with withGlobalTauri
// enabled, injected script could reach window.__TAURI__ and invoke backend
// commands. Cells are built with createElement/textContent throughout.

const invoke = window.__TAURI__.core.invoke;

const el = (id) => document.getElementById(id);

/** Currently displayed table, as returned by the backend. */
let table = null;
let sort = { index: null, ascending: true };
/** The dashboard view. Not a sheet — it has no table and is not exported. */
const INSIGHTS = "Environment Overview";

/** Sheet currently selected in the sidebar. */
let currentSheet = INSIGHTS;
/** Sheet names, in sidebar order. */
let sheets = [];
/** Row count per sheet once fetched, shown beside its sidebar entry. */
const rowCounts = new Map();

// ---- rendering ----

function setStatus(message) {
  el("status").textContent = message;
}

function renderWarnings(warnings) {
  const box = el("warnings");
  box.replaceChildren();
  if (!warnings || warnings.length === 0) {
    box.hidden = true;
    return;
  }
  box.hidden = false;
  const heading = document.createElement("strong");
  heading.textContent =
    warnings.length === 1
      ? "1 vCenter could not be queried — results below are incomplete:"
      : `${warnings.length} vCenters could not be queried — results below are incomplete:`;
  const list = document.createElement("ul");
  for (const w of warnings) {
    const li = document.createElement("li");
    li.textContent = w;
    list.append(li);
  }
  box.append(heading, list);
}

function cellText(value) {
  if (value === null || value === undefined) return "";
  if (typeof value === "boolean") return value ? "True" : "False";
  return String(value);
}

function renderHead() {
  const row = el("head-row");
  row.replaceChildren();
  table.columns.forEach((col, i) => {
    const th = document.createElement("th");
    th.textContent = col.label;
    th.title = col.label;
    if (sort.index === i) {
      th.classList.add("sorted");
      const arrow = document.createElement("span");
      arrow.className = "arrow";
      arrow.textContent = sort.ascending ? "▲" : "▼";
      th.append(arrow);
    }
    th.addEventListener("click", () => {
      sort = { index: i, ascending: sort.index === i ? !sort.ascending : true };
      renderHead();
      renderBody();
    });
    row.append(th);
  });
}

function sortedRows(rows) {
  if (sort.index === null) return rows;
  const i = sort.index;
  const numeric = table.columns[i].kind === "number";
  const dir = sort.ascending ? 1 : -1;

  return [...rows].sort((a, b) => {
    const x = a[i];
    const y = b[i];
    // Empty values sort last in both directions — a blank is "not reported",
    // not a small value.
    const xEmpty = x === null || x === undefined || x === "";
    const yEmpty = y === null || y === undefined || y === "";
    if (xEmpty || yEmpty) return xEmpty && yEmpty ? 0 : xEmpty ? 1 : -1;
    if (numeric) return (Number(x) - Number(y)) * dir;
    return String(x).localeCompare(String(y), undefined, { numeric: true }) * dir;
  });
}

function filteredRows(rows) {
  const needle = el("filter").value.trim().toLowerCase();
  if (!needle) return rows;
  return rows.filter((row) => row.some((v) => cellText(v).toLowerCase().includes(needle)));
}

function renderBody() {
  const body = el("body");
  body.replaceChildren();
  if (!table) return;

  const rows = sortedRows(filteredRows(table.rows));
  const frag = document.createDocumentFragment();

  for (const row of rows) {
    const tr = document.createElement("tr");
    row.forEach((value, i) => {
      const td = document.createElement("td");
      const text = cellText(value);
      td.textContent = text;
      td.title = text;
      if (table.columns[i].kind === "number") td.classList.add("num");
      if (typeof value === "boolean") td.classList.add(value ? "bool-true" : "bool-false");
      if (text === "") td.classList.add("empty");
      tr.append(td);
    });
    frag.append(tr);
  }
  body.append(frag);

  const shown = rows.length;
  const total = table.rows.length;
  setStatus(shown === total ? `${total} rows` : `${shown} of ${total} rows`);
}

// ---- dashboard ----

/** Number with thousands separators, or an em dash when absent. */
function num(value, digits = 0) {
  if (value === null || value === undefined || Number.isNaN(value)) return "—";
  return value.toLocaleString(undefined, {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  });
}

/** GiB rendered at a sensible scale. */
function size(gib) {
  if (gib === null || gib === undefined) return "—";
  if (gib >= 1024) return `${num(gib / 1024, 2)} TiB`;
  return `${num(gib, 1)} GiB`;
}

function make(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  // textContent throughout: datastore and cluster names are free text from
  // vCenter and must never reach innerHTML.
  if (text !== undefined) node.textContent = text;
  return node;
}

function kpi(label, value, unit, sub, tone = "hero") {
  const card = make("div", `card kpi ${tone}`);
  card.append(make("p", "kpi-label", label));
  const line = make("div", "kpi-value", value);
  if (unit) line.append(make("span", "kpi-unit", unit));
  card.append(line);
  if (sub) card.append(make("div", "kpi-sub", sub));
  return card;
}

/** Donut gauge for overall storage utilisation. */
function gauge(percent) {
  const svgNS = "http://www.w3.org/2000/svg";
  const size = 132;
  const r = 52;
  const circumference = 2 * Math.PI * r;
  const svg = document.createElementNS(svgNS, "svg");
  svg.setAttribute("class", "gauge");
  svg.setAttribute("width", size);
  svg.setAttribute("height", size);
  svg.setAttribute("viewBox", `0 0 ${size} ${size}`);

  const track = document.createElementNS(svgNS, "circle");
  track.setAttribute("class", "gauge-track");
  track.setAttribute("cx", size / 2);
  track.setAttribute("cy", size / 2);
  track.setAttribute("r", r);
  svg.append(track);

  const fill = document.createElementNS(svgNS, "circle");
  fill.setAttribute("class", "gauge-fill");
  fill.setAttribute("cx", size / 2);
  fill.setAttribute("cy", size / 2);
  fill.setAttribute("r", r);
  const clamped = Math.max(0, Math.min(100, percent));
  fill.setAttribute("stroke-dasharray", `${(clamped / 100) * circumference} ${circumference}`);
  fill.setAttribute("transform", `rotate(-90 ${size / 2} ${size / 2})`);
  if (clamped >= 90) fill.style.stroke = "var(--danger)";
  else if (clamped >= 75) fill.style.stroke = "var(--warning)";
  svg.append(fill);

  const pct = document.createElementNS(svgNS, "text");
  pct.setAttribute("class", "gauge-pct");
  pct.setAttribute("x", size / 2);
  pct.setAttribute("y", size / 2 + 4);
  pct.textContent = `${num(percent, 1)}%`;
  svg.append(pct);

  const cap = document.createElementNS(svgNS, "text");
  cap.setAttribute("class", "gauge-cap");
  cap.setAttribute("x", size / 2);
  cap.setAttribute("y", size / 2 + 20);
  cap.textContent = "USED";
  svg.append(cap);
  return svg;
}

function statLine(key, value) {
  const row = make("div", "stat");
  row.append(make("span", "k", key), make("span", "v", value));
  return row;
}

function barRow(name, valueText, percent, toneClass) {
  const row = make("div", "bar-row");
  row.append(make("span", "bar-name", name), make("span", "bar-val", valueText));
  const track = make("div", "bar-track");
  const fill = make("div", `bar-fill ${toneClass || ""}`);
  fill.style.width = `${Math.max(0, Math.min(100, percent))}%`;
  track.append(fill);
  row.append(track);
  return row;
}

function kindClass(kind) {
  const k = String(kind || "").toUpperCase();
  if (k.startsWith("NFS")) return "nfs";
  if (k.startsWith("VMFS")) return "vmfs";
  return "other";
}

function renderDashboard(i) {
  const board = el("dashboard");
  board.replaceChildren();

  // Headline tiles — the three the dashboard exists for, then context.
  const kpis = make("div", "kpi-row");
  kpis.append(
    kpi("Total Hosts", num(i.hosts), null,
      i.hosts_in_maintenance || i.hosts_disconnected
        ? `${i.hosts_in_maintenance} in maintenance · ${i.hosts_disconnected} disconnected`
        : `${i.clusters} clusters · all connected`),
    kpi("Total Cores", num(i.cores), null,
      `${num(i.vcpus)} vCPUs assigned`),
    kpi("Total Storage", size(i.storage_capacity_gib), null,
      `${size(i.storage_free_gib)} free across ${i.datastores} datastores`),
    kpi("Virtual Machines", num(i.vms_total), null, `${num(i.vms_powered_on)} powered on`, "muted"),
    kpi("Physical Memory", size(i.dram_gib), null,
      i.memory_total_gib > i.dram_gib ? `${size(i.memory_total_gib)} incl. memory tiers` : null, "muted"),
    kpi("vCPU : Core", i.vcpu_core_ratio ? `${num(i.vcpu_core_ratio, 2)}:1` : "—", null,
      `${size(i.vram_gib)} vRAM assigned`, "muted"),
  );
  board.append(kpis);

  // Storage utilisation + breakdown by backing type.
  const row = make("div", "panel-row");

  const gaugeCard = make("div", "card");
  gaugeCard.append(make("p", "card-title", "Storage utilisation"));
  const wrap = make("div", "gauge-wrap");
  wrap.append(gauge(i.storage_used_percent));
  const stats = make("div", "stat-list");
  stats.append(
    statLine("Capacity", size(i.storage_capacity_gib)),
    statLine("Used", size(i.storage_used_gib)),
    statLine("Free", size(i.storage_free_gib)),
    statLine("Datastores", num(i.datastores)),
  );
  wrap.append(stats);
  gaugeCard.append(wrap);
  row.append(gaugeCard);

  const typeCard = make("div", "card");
  typeCard.append(make("p", "card-title", "Capacity by datastore type"));
  const typeBars = make("div", "bars");
  const maxCapacity = Math.max(1, ...i.storage_by_type.map((t) => t.capacity_gib));
  for (const t of i.storage_by_type) {
    typeBars.append(
      barRow(
        `${t.kind} · ${t.datastores} datastore${t.datastores === 1 ? "" : "s"}`,
        `${size(t.used_gib)} of ${size(t.capacity_gib)}`,
        (t.capacity_gib / maxCapacity) * 100,
        kindClass(t.kind),
      ),
    );
  }
  typeCard.append(typeBars);
  row.append(typeCard);
  board.append(row);

  // Fullest datastores, then clusters.
  const row2 = make("div", "panel-row");

  const fullCard = make("div", "card");
  fullCard.append(make("p", "card-title", "Fullest datastores"));
  const fullBars = make("div", "bars");
  for (const d of i.top_datastores) {
    const tone = d.used_percent >= 90 ? "crit" : d.used_percent >= 75 ? "warn" : "";
    fullBars.append(
      barRow(d.name, `${num(d.used_percent, 1)}% · ${size(d.capacity_gib)}`, d.used_percent, tone),
    );
  }
  fullCard.append(fullBars);
  row2.append(fullCard);

  const clusterCard = make("div", "card");
  clusterCard.append(make("p", "card-title", "Clusters"));
  const table = make("table", "mini-table");
  const thead = make("thead");
  const hrow = make("tr");
  for (const [label, cls] of [["Cluster", ""], ["Hosts", "num"], ["Cores", "num"], ["DRAM", "num"]]) {
    hrow.append(make("th", cls, label));
  }
  thead.append(hrow);
  table.append(thead);
  const tbody = make("tbody");
  for (const c of i.cluster_summaries) {
    const tr = make("tr");
    tr.append(
      make("td", "", c.name),
      make("td", "num", num(c.hosts)),
      make("td", "num", num(c.cores)),
      make("td", "num", size(c.dram_gib)),
    );
    tbody.append(tr);
  }
  table.append(tbody);
  clusterCard.append(table);
  row2.append(clusterCard);
  board.append(row2);
}

async function loadInsights() {
  const button = el("refresh");
  button.disabled = true;
  el("sheet-title").textContent = INSIGHTS;
  setStatus("Building insights…");
  el("warnings").hidden = true;
  try {
    const insights = await invoke("fetch_insights");
    renderWarnings(insights.warnings);
    renderDashboard(insights);
    const servers = insights.servers.length;
    setStatus(`${insights.hosts} hosts · ${insights.cores} cores · ${size(insights.storage_capacity_gib)} across ${servers} vCenter${servers === 1 ? "" : "s"}`);
  } catch (e) {
    el("dashboard").replaceChildren();
    setStatus(String(e));
  } finally {
    button.disabled = false;
  }
}

// ---- data ----

function renderNav() {
  const nav = el("sheet-nav");
  nav.replaceChildren();
  for (const sheet of [INSIGHTS, ...sheets]) {
    const item = document.createElement("button");
    item.className = "nav-item" + (sheet === currentSheet ? " active" : "");
    item.type = "button";

    const label = document.createElement("span");
    label.textContent = sheet;
    item.append(label);

    // Row counts appear once a sheet has been fetched; an unvisited sheet shows
    // nothing rather than a misleading zero.
    if (rowCounts.has(sheet)) {
      const count = document.createElement("span");
      count.className = "count";
      count.textContent = rowCounts.get(sheet).toLocaleString();
      item.append(count);
    }

    item.addEventListener("click", () => {
      if (sheet === currentSheet) return;
      currentSheet = sheet;
      renderNav();
      loadSheet();
    });
    nav.append(item);
  }
}

/** Show either the dashboard or the table, never both. */
function setView(showDashboard) {
  const board = el("dashboard");
  board.hidden = !showDashboard;
  // Drop the old dashboard DOM when leaving it, so stale numbers can never be
  // shown again by a later toggle.
  if (!showDashboard) board.replaceChildren();
  el("table-wrap").hidden = showDashboard;
  // The filter box only applies to table rows.
  el("filter").disabled = showDashboard;
  el("filter").style.visibility = showDashboard ? "hidden" : "visible";
}

async function loadSheet() {
  if (currentSheet === INSIGHTS) {
    setView(true);
    return loadInsights();
  }
  setView(false);
  const button = el("refresh");
  button.disabled = true;
  el("sheet-title").textContent = currentSheet;
  setStatus(`Querying vCenter for ${currentSheet}…`);
  el("warnings").hidden = true;
  try {
    table = await invoke("fetch_sheet", { sheet: currentSheet });
    sort = { index: null, ascending: true };
    rowCounts.set(currentSheet, table.rows.length);
    renderNav();
    renderWarnings(table.warnings);
    renderHead();
    renderBody();
  } catch (e) {
    table = null;
    el("head-row").replaceChildren();
    el("body").replaceChildren();
    setStatus(String(e));
  } finally {
    button.disabled = false;
  }
}

// ---- export ----

async function exportXlsx() {
  const button = el("export");
  button.disabled = true;
  const previous = el("status").textContent;
  setStatus("Collecting every sheet for export…");
  try {
    const result = await invoke("export_xlsx");
    if (!result.path) {
      setStatus(previous);
      return;
    }
    // Warnings ride along with the export: a workbook missing a vCenter's rows
    // must say so, not just report a row count.
    renderWarnings(result.warnings);
    const sheets = `${result.sheets} sheet${result.sheets === 1 ? "" : "s"}`;
    setStatus(`Exported ${sheets}, ${result.rows.toLocaleString()} rows → ${result.path}`);
  } catch (e) {
    setStatus(String(e));
  } finally {
    button.disabled = false;
  }
}

/// Host + storage topology as a standalone HTML file.
async function exportReport() {
  const button = el("report");
  button.disabled = true;
  const previous = el("status").textContent;
  setStatus("Building the topology report…");
  try {
    const result = await invoke("export_topology_report");
    if (!result.path) {
      setStatus(previous);
      return;
    }
    renderWarnings(result.warnings);
    const hosts = `${result.hosts} host${result.hosts === 1 ? "" : "s"}`;
    const stores = `${result.datastores} datastore${result.datastores === 1 ? "" : "s"}`;
    setStatus(`Topology report: ${hosts}, ${stores} → ${result.path}`);
  } catch (e) {
    setStatus(String(e));
  } finally {
    button.disabled = false;
  }
}

// ---- settings ----

function connectionRow(conn = { host: "", username: "", password: "", skip_cert_verify: true }) {
  const wrap = document.createElement("div");
  wrap.className = "connection";

  const host = document.createElement("input");
  host.type = "text";
  host.placeholder = "vcsa.example.com";
  host.value = conn.host;
  host.dataset.field = "host";

  const user = document.createElement("input");
  user.type = "text";
  user.placeholder = "administrator@vsphere.local";
  user.value = conn.username;
  user.dataset.field = "username";

  const pass = document.createElement("input");
  pass.type = "password";
  pass.placeholder = "Password";
  pass.value = conn.password;
  pass.dataset.field = "password";

  const remove = document.createElement("button");
  remove.type = "button";
  remove.textContent = "Remove";
  remove.addEventListener("click", () => wrap.remove());

  wrap.append(host, user, pass, remove);
  return wrap;
}

function readConnections() {
  return [...el("connections").querySelectorAll(".connection")]
    .map((row) => {
      const field = (name) => row.querySelector(`[data-field="${name}"]`).value.trim();
      return {
        host: field("host"),
        username: field("username"),
        password: row.querySelector('[data-field="password"]').value,
        skip_cert_verify: true,
      };
    })
    .filter((c) => c.host !== "");
}

async function openSettings() {
  const container = el("connections");
  container.replaceChildren();
  el("settings-status").textContent = "";
  try {
    const cfg = await invoke("get_config");
    const conns = cfg.connections.length > 0 ? cfg.connections : [undefined];
    for (const c of conns) container.append(connectionRow(c));
  } catch (e) {
    el("settings-status").textContent = String(e);
    container.append(connectionRow());
  }
  el("settings").showModal();
}

async function saveSettings() {
  const status = el("settings-status");
  status.textContent = "Saving…";
  try {
    await invoke("save_config", { cfg: { connections: readConnections() } });
    el("settings").close();
    await loadSheet();
  } catch (e) {
    status.textContent = String(e);
  }
}

// ---- wiring ----

el("refresh").addEventListener("click", loadSheet);
el("filter").addEventListener("input", renderBody);
el("export").addEventListener("click", exportXlsx);
el("report").addEventListener("click", exportReport);
el("open-settings").addEventListener("click", openSettings);
el("add-connection").addEventListener("click", () => el("connections").append(connectionRow()));
el("save-settings").addEventListener("click", saveSettings);

(async function start() {
  sheets = await invoke("list_sheets").catch(() => ["vInfo"]);
  el("sheet-title").textContent = currentSheet;
  renderNav();
  const cfg = await invoke("get_config").catch(() => ({ connections: [] }));
  if (cfg.connections.length === 0) {
    setStatus("No vCenter configured yet — open Settings to add one.");
    return;
  }
  await loadSheet();
})();
