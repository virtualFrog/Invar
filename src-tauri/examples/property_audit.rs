//! Audit every property path the app declares against a live vCenter.
//!
//! Ground rule 1 in `CLAUDE.md` says no property path is written without being
//! queried live first. That is a rule about the moment code is written; this is
//! the same rule applied continuously, because a vCenter upgrade can retire a
//! path that was verified perfectly well against the previous build. A path
//! that stops returning does not error — it silently empties a column.
//!
//! Run it after any vCenter upgrade:
//!
//! ```text
//! cargo run --example property_audit
//! ```
//!
//! It reads the property sets straight off `data::SHEETS`, so it audits
//! whatever the app currently asks for, with no list to keep in sync. Output is
//! per property: how many objects returned it, and which sheets asked for it.
//! Exit status is non-zero if anything returned for nothing, so it can gate CI.

use std::collections::{BTreeMap, BTreeSet};
use sttools_lib::data::{self, snapshot::union};
use sttools_lib::vcenter::{config, SessionCache};

/// Which sheets asked for a given property, per object type.
fn askers(
    get: fn(&data::snapshot::SheetSpec) -> &'static [&'static [&'static str]],
) -> BTreeMap<&'static str, BTreeSet<&'static str>> {
    let mut out: BTreeMap<&'static str, BTreeSet<&'static str>> = BTreeMap::new();
    for spec in data::SHEETS {
        for set in get(spec) {
            for prop in *set {
                out.entry(prop).or_default().insert(spec.name);
            }
        }
    }
    out
}

fn sets(
    get: fn(&data::snapshot::SheetSpec) -> &'static [&'static [&'static str]],
) -> Vec<&'static [&'static str]> {
    data::SHEETS.iter().flat_map(|s| get(s).iter().copied()).collect()
}

#[tokio::main]
async fn main() {
    let dir = std::env::var("APPDATA")
        .map(|a| std::path::PathBuf::from(a).join("ch.soultec.sttools"))
        .expect("APPDATA must be set");
    let cfg = config::load(&config::config_path(dir)).expect("config loads");
    let conn = cfg.connections.first().expect("a configured vCenter").clone();

    let cache = SessionCache::new();
    let session = match cache.get(&conn).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("could not connect to {}: {e}", conn.label());
            std::process::exit(2);
        }
    };

    // The build being audited against, so a report is attributable.
    match session.soap.service_content().await {
        Ok(sc) => {
            let about = sc.child("about");
            eprintln!(
                "auditing {} ({})",
                conn.label(),
                about
                    .and_then(|a| a.text_at("fullName"))
                    .unwrap_or_else(|| "unknown build".into())
            );
        }
        Err(e) => eprintln!("warning: could not read ServiceContent: {e}"),
    }
    eprintln!();

    let groups: [(&str, Vec<&[&str]>, BTreeMap<&str, BTreeSet<&str>>); 5] = [
        ("VirtualMachine", sets(|s| s.vm_props), askers(|s| s.vm_props)),
        ("HostSystem", sets(|s| s.host_props), askers(|s| s.host_props)),
        ("DistributedVirtualSwitch", sets(|s| s.dvs_props), askers(|s| s.dvs_props)),
        (
            "DistributedVirtualPortgroup",
            sets(|s| s.dvpg_props),
            askers(|s| s.dvpg_props),
        ),
        ("ClusterComputeResource", sets(|s| s.cluster_props), askers(|s| s.cluster_props)),
    ];

    let mut dead: Vec<(String, String, String)> = Vec::new();
    let mut audited = 0usize;

    for (obj_type, prop_sets, who) in groups {
        let props = union(&prop_sets);
        if props.is_empty() {
            continue;
        }
        let objects = match session.soap.retrieve(obj_type, &props).await {
            Ok(o) => o,
            Err(e) => {
                eprintln!("{obj_type}: retrieve failed: {e}");
                continue;
            }
        };
        println!("=== {obj_type}: {} objects, {} properties ===", objects.len(), props.len());
        for prop in &props {
            audited += 1;
            let n = objects.iter().filter(|o| o.prop(prop).is_some()).count();
            let asked = who
                .get(prop)
                .map(|s| s.iter().copied().collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            if n == 0 {
                println!("  {prop:<52} 0/{:<5} NEVER RETURNED   [{asked}]", objects.len());
                dead.push((obj_type.to_string(), prop.to_string(), asked));
            } else if n < objects.len() {
                println!("  {prop:<52} {n}/{:<5} partial", objects.len());
            } else {
                println!("  {prop:<52} {n}/{:<5} ok", objects.len());
            }
        }
        println!();
    }

    // The two singletons, neither of which is reachable by a ContainerView.
    match session.soap.service_content().await {
        Ok(sc) if sc.child("about").is_some() => println!("ServiceContent.about        present (vSource)"),
        _ => {
            println!("ServiceContent.about        MISSING (vSource)");
            dead.push(("ServiceInstance".into(), "about".into(), "vSource".into()));
        }
    }
    match session.soap.retrieve_moref("LicenseManager", "LicenseManager", &["licenses"]).await {
        Ok(Some(lm)) if !lm.array_prop("licenses").is_empty() => {
            println!("LicenseManager.licenses     {} entries (vLicense)", lm.array_prop("licenses").len())
        }
        _ => {
            println!("LicenseManager.licenses     MISSING or empty (vLicense)");
            dead.push(("LicenseManager".into(), "licenses".into(), "vLicense".into()));
        }
    }

    println!("\n{audited} properties audited.");
    if dead.is_empty() {
        println!("All of them returned for at least one object.");
    } else {
        println!("\n{} path(s) returned for NOTHING — each is a silently empty column:", dead.len());
        for (ty, prop, asked) in &dead {
            println!("  {ty}.{prop}   asked for by: {asked}");
        }
        println!("\nA path can be legitimately absent (a feature not in use) or retired by an");
        println!("upgrade. Check each against the lab before changing code.");
    }

    cache.close_all().await;
    if !dead.is_empty() {
        std::process::exit(1);
    }
}
