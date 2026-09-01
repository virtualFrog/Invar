//! Self-contained HTML topology report.
//!
//! Everything is inlined — no external CSS, fonts or scripts — so the file can
//! be emailed or archived and still render years later. The diagram is an SVG
//! whose coordinates are computed here rather than by script in the page, so it
//! is correct even where JavaScript is blocked.
//!
//! vCenter names are free text and are HTML-escaped everywhere they appear.

use crate::data::topology::{DatastoreNode, ServerTopology, Topology};

/// Escape text for HTML. Datastore and VM names are operator-supplied, so
/// interpolating them raw would be an injection into whatever opens the report.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn gib(v: Option<f64>) -> String {
    v.map(|n| {
        if n >= 1024.0 {
            format!("{:.2} TiB", n / 1024.0)
        } else {
            format!("{n:.1} GiB")
        }
    })
    .unwrap_or_else(|| "—".into())
}

/// Datastore colour by backing type, matching the diagram legend.
fn kind_class(kind: Option<&str>) -> &'static str {
    match kind.map(str::to_ascii_uppercase).as_deref() {
        Some(k) if k.starts_with("VMFS") => "vmfs",
        Some(k) if k.starts_with("NFS") => "nfs",
        Some(k) if k.starts_with("VSAN") => "vsan",
        Some(k) if k.starts_with("VVOL") => "vvol",
        _ => "other",
    }
}

// ---- diagram geometry ----

const HOST_H: f64 = 52.0;
const HOST_GAP: f64 = 12.0;
const DS_H: f64 = 62.0;
const DS_GAP: f64 = 12.0;
const CLUSTER_PAD: f64 = 30.0;
const CLUSTER_GAP: f64 = 22.0;
const NODE_W: f64 = 250.0;
const COL_GAP: f64 = 210.0;
const MARGIN: f64 = 24.0;

/// Lay out one vCenter's hosts (left, grouped by cluster) and datastores
/// (right), then connect each host to the datastores it has mounted.
fn diagram(t: &ServerTopology) -> String {
    let host_x = MARGIN;
    let ds_x = MARGIN + NODE_W + COL_GAP;
    let width = ds_x + NODE_W + MARGIN;

    // Left column: cluster boxes stacked, hosts inside them.
    let mut host_positions: Vec<(String, f64)> = Vec::new(); // moref → centre y
    let mut left = String::new();
    let mut y = MARGIN + 26.0;

    let mut groups: Vec<(String, Vec<&crate::data::topology::HostNode>)> = t
        .clusters
        .iter()
        .map(|(name, hosts)| (name.clone(), hosts.iter().collect()))
        .collect();
    if !t.standalone_hosts.is_empty() {
        groups.push((
            "Standalone hosts".into(),
            t.standalone_hosts.iter().collect(),
        ));
    }

    for (cluster, hosts) in &groups {
        let inner = if hosts.is_empty() {
            HOST_H
        } else {
            hosts.len() as f64 * HOST_H + (hosts.len() as f64 - 1.0) * HOST_GAP
        };
        let box_h = inner + CLUSTER_PAD + 14.0;
        left.push_str(&format!(
            r#"<g class="cluster"><rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="8"/>
<text class="cluster-label" x="{tx:.1}" y="{ty:.1}">{name}</text></g>"#,
            x = host_x - 14.0,
            y = y - 24.0,
            w = NODE_W + 28.0,
            h = box_h,
            tx = host_x - 2.0,
            ty = y - 7.0,
            name = esc(cluster),
        ));

        let mut hy = y + 8.0;
        for host in hosts {
            let state = if host.in_maintenance {
                "maint"
            } else if host.connection_state.as_deref() == Some("connected") {
                "ok"
            } else {
                "bad"
            };
            // With memory tiering on, `memorySize` counts the NVMe tier too, so
            // the node shows DRAM — the figure an operator reads as "RAM".
            let detail = match host.dram_gib {
                Some(dram) => format!(
                    "{} cores · {} DRAM",
                    host.cpu_cores.map(|c| c.to_string()).unwrap_or_else(|| "—".into()),
                    gib(Some(dram))
                ),
                None => format!(
                    "{} cores · {}",
                    host.cpu_cores.map(|c| c.to_string()).unwrap_or_else(|| "—".into()),
                    gib(host.memory_gib)
                ),
            };
            left.push_str(&format!(
                r#"<g class="node host {state}" data-host="{id}"><rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="6"/>
<circle class="dot" cx="{cx:.1}" cy="{cy:.1}" r="4"/>
<text class="node-title" x="{tx:.1}" y="{t1:.1}">{name}</text>
<text class="node-sub" x="{tx:.1}" y="{t2:.1}">{detail}</text></g>"#,
                state = state,
                id = esc(&host.moref),
                x = host_x,
                y = hy,
                w = NODE_W,
                h = HOST_H,
                cx = host_x + 16.0,
                cy = hy + HOST_H / 2.0,
                tx = host_x + 30.0,
                t1 = hy + 21.0,
                t2 = hy + 38.0,
                name = esc(&host.name),
                detail = esc(&detail),
            ));
            host_positions.push((host.moref.clone(), hy + HOST_H / 2.0));
            hy += HOST_H + HOST_GAP;
        }
        y += box_h + CLUSTER_GAP;
    }
    let left_height = y;

    // Right column: datastores.
    let mut ds_positions: Vec<(String, f64)> = Vec::new();
    let mut right = String::new();
    let mut dy = MARGIN + 26.0;
    for ds in &t.datastores {
        let class = kind_class(ds.kind.as_deref());
        let used_pct = ds.used_percent().unwrap_or(0.0);
        let bar_w = (NODE_W - 32.0) * (used_pct / 100.0).clamp(0.0, 1.0);
        let sub = format!(
            "{} · {} free of {}",
            ds.kind.clone().unwrap_or_else(|| "—".into()),
            gib(ds.free_gib),
            gib(ds.capacity_gib)
        );
        right.push_str(&format!(
            r#"<g class="node ds {class}" data-ds="{id}"><rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="6"/>
<rect class="kind-bar" x="{x:.1}" y="{y:.1}" width="5" height="{h:.1}"/>
<text class="node-title" x="{tx:.1}" y="{t1:.1}">{name}</text>
<text class="node-sub" x="{tx:.1}" y="{t2:.1}">{sub}</text>
<rect class="cap-track" x="{tx:.1}" y="{t3:.1}" width="{track:.1}" height="5" rx="2.5"/>
<rect class="cap-fill" x="{tx:.1}" y="{t3:.1}" width="{bar:.1}" height="5" rx="2.5"/></g>"#,
            class = class,
            id = esc(&ds.moref),
            x = ds_x,
            y = dy,
            w = NODE_W,
            h = DS_H,
            tx = ds_x + 16.0,
            t1 = dy + 20.0,
            t2 = dy + 36.0,
            t3 = dy + 46.0,
            track = NODE_W - 32.0,
            bar = bar_w,
            name = esc(&ds.name),
            sub = esc(&sub),
        ));
        ds_positions.push((ds.moref.clone(), dy + DS_H / 2.0));
        dy += DS_H + DS_GAP;
    }
    let right_height = dy;

    // Links, drawn first so nodes paint over them.
    let mut links = String::new();
    for ds in &t.datastores {
        let Some((_, dsy)) = ds_positions.iter().find(|(m, _)| *m == ds.moref) else {
            continue;
        };
        let class = kind_class(ds.kind.as_deref());
        for host_moref in &ds.mounted_by {
            let Some((_, hy)) = host_positions.iter().find(|(m, _)| m == host_moref) else {
                // A datastore can be mounted by a host outside this vCenter's
                // inventory view; skip rather than drawing a line to nowhere.
                continue;
            };
            let x1 = host_x + NODE_W;
            let x2 = ds_x;
            let mid = (x1 + x2) / 2.0;
            links.push_str(&format!(
                r#"<path class="link {class}" data-host="{h}" data-ds="{d}" d="M{x1:.1},{y1:.1} C{mid:.1},{y1:.1} {mid:.1},{y2:.1} {x2:.1},{y2:.1}"/>"#,
                class = class,
                h = esc(host_moref),
                d = esc(&ds.moref),
                x1 = x1,
                y1 = hy,
                x2 = x2,
                y2 = dsy,
                mid = mid,
            ));
        }
    }

    let height = left_height.max(right_height) + MARGIN;
    format!(
        r#"<svg class="topology" viewBox="0 0 {width:.0} {height:.0}" role="img" aria-label="Host and storage topology">
<text class="col-head" x="{hx:.1}" y="18">HOSTS</text>
<text class="col-head" x="{dx:.1}" y="18">DATASTORES</text>
<g class="links">{links}</g>{left}{right}</svg>"#,
        width = width,
        height = height,
        hx = host_x,
        dx = ds_x,
    )
}

fn datastore_rows(t: &ServerTopology) -> String {
    let host_name = |moref: &str| {
        t.all_hosts()
            .into_iter()
            .find(|h| h.moref == moref)
            .map(|h| h.name.clone())
            .unwrap_or_else(|| moref.to_string())
    };

    t.datastores
        .iter()
        .map(|ds| {
            let mut mounts: Vec<String> = ds.mounted_by.iter().map(|m| host_name(m)).collect();
            mounts.sort();
            let pct = ds.used_percent();
            format!(
                r#"<tr><td>{name}</td><td><span class="tag {class}">{kind}</span></td>
<td class="num">{cap}</td><td class="num">{used}</td><td class="num">{free}</td>
<td class="num">{pct}</td><td class="num">{vms}</td><td class="num">{nmounts}</td><td class="hosts">{mounts}</td></tr>"#,
                name = esc(&ds.name),
                class = kind_class(ds.kind.as_deref()),
                kind = esc(ds.kind.as_deref().unwrap_or("—")),
                cap = gib(ds.capacity_gib),
                used = gib(ds.used_gib()),
                free = gib(ds.free_gib),
                pct = pct.map(|p| format!("{p:.1}%")).unwrap_or_else(|| "—".into()),
                vms = ds.vm_count,
                nmounts = ds.mounted_by.len(),
                mounts = esc(&mounts.join(", ")),
            )
        })
        .collect()
}

fn host_rows(t: &ServerTopology) -> String {
    t.all_hosts()
        .into_iter()
        .map(|h| {
            let mounted: Vec<&DatastoreNode> = t
                .datastores
                .iter()
                .filter(|d| d.mounted_by.contains(&h.moref))
                .collect();
            let names: Vec<String> = mounted.iter().map(|d| esc(&d.name)).collect();
            format!(
                r#"<tr><td>{name}</td><td>{cluster}</td><td>{state}</td><td class="num">{cores}</td>
<td class="num">{dram}</td><td class="num">{mem}</td><td class="num">{n}</td><td class="hosts">{list}</td></tr>"#,
                name = esc(&h.name),
                cluster = esc(h.cluster.as_deref().unwrap_or("—")),
                state = esc(if h.in_maintenance {
                    "maintenance"
                } else {
                    h.connection_state.as_deref().unwrap_or("—")
                }),
                cores = h.cpu_cores.map(|c| c.to_string()).unwrap_or_else(|| "—".into()),
                dram = gib(h.dram_gib),
                mem = gib(h.memory_gib),
                n = mounted.len(),
                list = names.join(", "),
            )
        })
        .collect()
}

fn server_section(t: &ServerTopology) -> String {
    let hosts = t.all_hosts().len();
    let capacity: f64 = t.datastores.iter().filter_map(|d| d.capacity_gib).sum();
    let free: f64 = t.datastores.iter().filter_map(|d| d.free_gib).sum();

    format!(
        r#"<section class="server">
<h2>{server}</h2>
<p class="meta">{dcs} · {clusters} clusters · {hosts} hosts · {ds} datastores · {cap} capacity, {free} free</p>
<div class="legend">
  <span><i class="swatch vmfs"></i>VMFS</span><span><i class="swatch nfs"></i>NFS</span>
  <span><i class="swatch vsan"></i>vSAN</span><span><i class="swatch vvol"></i>vVol</span>
  <span><i class="swatch other"></i>Other</span>
</div>
<div class="diagram">{diagram}</div>
<h3>Datastores</h3>
<div class="table-wrap"><table>
<thead><tr><th>Datastore</th><th>Type</th><th>Capacity</th><th>Used</th><th>Free</th><th>Used %</th><th>VMs</th><th>Mounts</th><th>Mounted by</th></tr></thead>
<tbody>{ds_rows}</tbody></table></div>
<h3>Hosts</h3>
<div class="table-wrap"><table>
<thead><tr><th>Host</th><th>Cluster</th><th>State</th><th>Cores</th><th>DRAM</th><th>Memory (incl. tiers)</th><th>Datastores</th><th>Mounted datastores</th></tr></thead>
<tbody>{host_rows}</tbody></table></div>
</section>"#,
        server = esc(&t.server),
        dcs = if t.datacenters.is_empty() {
            "no datacenter".to_string()
        } else {
            esc(&t.datacenters.join(", "))
        },
        clusters = t.clusters.len(),
        hosts = hosts,
        ds = t.datastores.len(),
        cap = gib(Some(capacity)),
        free = gib(Some(free)),
        diagram = diagram(t),
        ds_rows = datastore_rows(t),
        host_rows = host_rows(t),
    )
}

const STYLE: &str = r#"
:root{--bg:#1b2a32;--panel:#22343c;--panel-alt:#1e2f37;--line:#314351;--line-2:#495a63;
--text:#eaedf0;--dim:#adbbc4;--faint:#798d99;--accent:#49afd9;--warn:#f5c348;
--vmfs:#49afd9;--nfs:#60b515;--vsan:#c47bd6;--vvol:#f5c348;--other:#798d99;}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--text);
font:14px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;-webkit-font-smoothing:antialiased}
header{padding:22px 28px;border-bottom:1px solid var(--line);background:var(--panel-alt)}
h1{margin:0;font-size:19px;font-weight:600}
header .meta{margin:4px 0 0;color:var(--dim);font-size:12px}
main{padding:24px 28px 48px;max-width:1400px}
section.server{margin-bottom:44px}
h2{font-size:16px;margin:0 0 4px;color:var(--accent)}
h3{font-size:13px;text-transform:uppercase;letter-spacing:.07em;color:var(--faint);margin:26px 0 8px}
p.meta{margin:0 0 14px;color:var(--dim);font-size:12px}
.warnings{margin:0 0 20px;padding:10px 14px;border-left:3px solid var(--warn);
background:rgba(245,195,72,.1);border-radius:3px}
.warnings ul{margin:6px 0 0;padding-left:18px;color:var(--dim)}
.legend{display:flex;gap:16px;flex-wrap:wrap;margin-bottom:10px;color:var(--dim);font-size:12px}
.legend span{display:flex;align-items:center;gap:6px}
.swatch{width:10px;height:10px;border-radius:2px;display:inline-block}
.swatch.vmfs{background:var(--vmfs)}.swatch.nfs{background:var(--nfs)}
.swatch.vsan{background:var(--vsan)}.swatch.vvol{background:var(--vvol)}
.swatch.other{background:var(--other)}
.diagram{background:var(--panel);border:1px solid var(--line);border-radius:4px;padding:8px;overflow-x:auto}
svg.topology{display:block;width:100%;height:auto;min-width:760px}
.col-head{fill:var(--faint);font-size:10px;letter-spacing:.09em;font-weight:600}
.cluster rect{fill:rgba(73,175,217,.04);stroke:var(--line-2);stroke-dasharray:3 3}
.cluster-label{fill:var(--dim);font-size:11px;font-weight:600}
.node rect{fill:var(--panel-alt);stroke:var(--line-2)}
.node-title{fill:var(--text);font-size:12.5px;font-weight:600}
.node-sub{fill:var(--faint);font-size:11px}
.host .dot{fill:var(--nfs)}
.host.maint .dot{fill:var(--vvol)}
.host.bad .dot{fill:#f54f47}
.kind-bar{stroke:none}
.ds.vmfs .kind-bar{fill:var(--vmfs)}.ds.nfs .kind-bar{fill:var(--nfs)}
.ds.vsan .kind-bar{fill:var(--vsan)}.ds.vvol .kind-bar{fill:var(--vvol)}
.ds.other .kind-bar{fill:var(--other)}
.cap-track{fill:#16242b}
.cap-fill{fill:var(--accent)}
.link{fill:none;stroke-width:1.4;opacity:.42}
.link.vmfs{stroke:var(--vmfs)}.link.nfs{stroke:var(--nfs)}
.link.vsan{stroke:var(--vsan)}.link.vvol{stroke:var(--vvol)}.link.other{stroke:var(--other)}
svg.topology.focus .link{opacity:.08}
svg.topology.focus .link.on{opacity:1;stroke-width:2.4}
svg.topology.focus .node{opacity:.35}
svg.topology.focus .node.on{opacity:1}
.node{cursor:pointer}
.table-wrap{overflow-x:auto;border:1px solid var(--line);border-radius:4px;background:var(--panel)}
table{border-collapse:collapse;width:100%;font-size:12.5px}
th,td{padding:7px 12px;text-align:left;border-bottom:1px solid var(--line);white-space:nowrap}
th{background:#17262e;color:var(--dim);font-size:10.5px;text-transform:uppercase;letter-spacing:.06em}
tbody tr:nth-child(even){background:#1f313a}
td.num{text-align:right;font-variant-numeric:tabular-nums}
td.hosts{white-space:normal;color:var(--dim);min-width:220px}
.tag{padding:1px 7px;border-radius:9px;font-size:11px;font-weight:600;color:#0b1a21}
.tag.vmfs{background:var(--vmfs)}.tag.nfs{background:var(--nfs)}
.tag.vsan{background:var(--vsan)}.tag.vvol{background:var(--vvol)}.tag.other{background:var(--other)}
footer{padding:18px 28px;border-top:1px solid var(--line);color:var(--faint);font-size:11.5px}
@media print{body{background:#fff;color:#000}.diagram,.table-wrap{background:#fff}}
"#;

/// Hover highlighting only — the diagram is fully laid out without it, so the
/// report is complete with scripting disabled.
const SCRIPT: &str = r#"
document.querySelectorAll('svg.topology').forEach(function (svg) {
  function clear() {
    svg.classList.remove('focus');
    svg.querySelectorAll('.on').forEach(function (el) { el.classList.remove('on'); });
  }
  svg.querySelectorAll('.node').forEach(function (node) {
    node.addEventListener('mouseenter', function () {
      clear();
      var host = node.getAttribute('data-host');
      var ds = node.getAttribute('data-ds');
      var sel = host ? '[data-host="' + host + '"]' : '[data-ds="' + ds + '"]';
      svg.classList.add('focus');
      node.classList.add('on');
      svg.querySelectorAll('.link' + sel).forEach(function (link) {
        link.classList.add('on');
        var other = host ? link.getAttribute('data-ds') : link.getAttribute('data-host');
        var attr = host ? 'data-ds' : 'data-host';
        var peer = svg.querySelector('.node[' + attr + '="' + other + '"]');
        if (peer) { peer.classList.add('on'); }
      });
    });
    node.addEventListener('mouseleave', clear);
  });
});
"#;

/// Render the whole report.
pub fn render(topology: &Topology) -> String {
    let generated = chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z").to_string();

    let warnings = if topology.warnings.is_empty() {
        String::new()
    } else {
        format!(
            r#"<div class="warnings"><strong>{n} vCenter{s} could not be queried — this report is incomplete:</strong><ul>{items}</ul></div>"#,
            n = topology.warnings.len(),
            s = if topology.warnings.len() == 1 { "" } else { "s" },
            items = topology
                .warnings
                .iter()
                .map(|w| format!("<li>{}</li>", esc(w)))
                .collect::<String>(),
        )
    };

    let body = if topology.servers.is_empty() {
        "<p class=\"meta\">No vCenter returned any topology data.</p>".to_string()
    } else {
        topology.servers.iter().map(server_section).collect()
    };

    let servers = topology
        .servers
        .iter()
        .map(|s| esc(&s.server))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>vCenter Topology Report</title>
<style>{STYLE}</style></head>
<body>
<header>
  <h1>Host &amp; Storage Topology</h1>
  <p class="meta">{servers} · generated {generated} by {tool} {version}</p>
</header>
<main>{warnings}{body}</main>
<footer>Lines connect each host to the datastores it has mounted. Hover a host or datastore to isolate its connections.</footer>
<script>{SCRIPT}</script>
</body></html>"#,
        servers = if servers.is_empty() { "No vCenter".into() } else { servers },
        tool = env!("CARGO_PKG_NAME"),
        version = env!("CARGO_PKG_VERSION"),
    )
}

/// Default filename, in the same spirit as the xlsx export.
pub fn default_filename() -> String {
    format!(
        "vCenter_Topology_{}.html",
        chrono::Local::now().format("%Y-%m-%d_%H.%M.%S")
    )
}
