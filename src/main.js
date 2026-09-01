// vCenter Inventory — frontend.
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

// ---- data ----

async function loadSheet() {
  const button = el("refresh");
  button.disabled = true;
  setStatus("Querying vCenter…");
  el("warnings").hidden = true;
  try {
    table = await invoke("fetch_vinfo");
    sort = { index: null, ascending: true };
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
el("open-settings").addEventListener("click", openSettings);
el("add-connection").addEventListener("click", () => el("connections").append(connectionRow()));
el("save-settings").addEventListener("click", saveSettings);

(async function start() {
  const cfg = await invoke("get_config").catch(() => ({ connections: [] }));
  if (cfg.connections.length === 0) {
    setStatus("No vCenter configured yet — open Settings to add one.");
    return;
  }
  await loadSheet();
})();
