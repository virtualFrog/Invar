//! Count the vCenter sessions this tool has left behind.
//!
//! vCenter sessions do not clean themselves up promptly — they linger until a
//! roughly 30 minute idle timeout. Logging in per API call leaks them fast, and
//! an earlier version of this app accumulated some 300 open sessions in a day
//! of testing. `SessionCache` exists to stop that, but nothing proved it was
//! working: the leak is invisible from inside the app, and vCenter reports no
//! error when it happens.
//!
//! ```text
//! cargo run --example session_audit
//! cargo run --example session_audit -- --all        # list every session
//! cargo run --example session_audit -- --max 5      # exit 1 above 5 of ours
//! ```
//!
//! The intended use is a bracket, because a count on its own says nothing:
//!
//! ```text
//! cargo run --example session_audit            # note "ours"
//! cargo run --bin invar-export -- --xlsx /tmp/x.xlsx
//! cargo run --example session_audit            # "ours" should not have grown
//! ```
//!
//! "Ours" means sessions whose user matches the configured connection, which is
//! the closest attribution available: the app sets no distinguishing user agent,
//! so a session opened by Invar and one opened by a browser logged in as the
//! same administrator are indistinguishable here. Run it against a vCenter
//! nobody else is using, or read the count as an upper bound.
//!
//! Note that a REST login also produces a server-side `vapi-endpoint` session
//! from `127.0.0.1`, so one Invar connection can show up as more than one row.
//!
//! Idle time is reported only when the server actually supplies it. vCenter
//! does not reliably advance `lastActiveTime` — on the 9.1.1.0 lab most
//! sessions return it equal to `loginTime`, which makes idle time a copy of
//! age. The audit measures that proportion per run and says so, and suppresses
//! the idle figure entirely when no session contradicts it.
//!
//! This audit opens a session of its own. It closes it before exiting, and
//! excludes it from the totals so the number does not include the observer.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use invar_lib::vcenter::soap::ManagedObject;
use invar_lib::vcenter::{config, SessionCache, VCenterConnection};

/// One row of `SessionManager.sessionList`.
struct UserSession {
    key: String,
    user_name: String,
    login_time: Option<DateTime<Utc>>,
    last_active: Option<DateTime<Utc>>,
    ip_address: String,
    user_agent: String,
}

impl UserSession {
    /// Minutes since the session last made a call. vCenter expires a session
    /// at roughly 30 idle minutes, so this is how close one is to going away
    /// on its own.
    fn idle_minutes(&self, now: DateTime<Utc>) -> Option<i64> {
        self.last_active.map(|t| (now - t).num_minutes())
    }

    /// Minutes since login. Read alongside idle time: a session that is old
    /// *and* barely idle is being kept alive by something still running, which
    /// is what a reused cached session looks like. Old and nearly expired is
    /// the shape of one that was abandoned.
    fn age_minutes(&self, now: DateTime<Utc>) -> Option<i64> {
        self.login_time.map(|t| (now - t).num_minutes())
    }
}

fn parse_time(s: Option<String>) -> Option<DateTime<Utc>> {
    s.and_then(|t| DateTime::parse_from_rfc3339(&t).ok())
        .map(|t| t.with_timezone(&Utc))
}

/// Read `sessionList` off the SessionManager.
///
/// Verified live 2026-09-10: `sessionList` comes back as
/// `<val xsi:type="ArrayOfUserSession">` whose children are named
/// `UserSession` — the declared element *type*, not the field name, which is
/// the vim25 array rule in `CLAUDE.md`. Reading `<sessionList>` children here
/// would yield zero rows and no error.
fn sessions_of(mo: &ManagedObject) -> Vec<UserSession> {
    let Some(list) = mo.prop("sessionList") else {
        return Vec::new();
    };
    list.children_named("UserSession")
        .map(|s| UserSession {
            key: s.text_at("key").unwrap_or_default(),
            user_name: s.text_at("userName").unwrap_or_default(),
            login_time: parse_time(s.text_at("loginTime")),
            last_active: parse_time(s.text_at("lastActiveTime")),
            ip_address: s.text_at("ipAddress").unwrap_or_default(),
            user_agent: s.text_at("userAgent").unwrap_or_default(),
        })
        .collect()
}

/// vCenter reports `VSPHERE.LOCAL\Administrator` where the config says
/// `administrator@vsphere.local`. Compare on the two halves, case-insensitively,
/// so "ours" actually matches.
fn same_user(session_user: &str, configured: &str) -> bool {
    fn parts(s: &str) -> (String, String) {
        let s = s.to_lowercase();
        if let Some((domain, user)) = s.split_once('\\') {
            (user.to_string(), domain.to_string())
        } else if let Some((user, domain)) = s.split_once('@') {
            (user.to_string(), domain.to_string())
        } else {
            (s, String::new())
        }
    }
    let (au, ad) = parts(session_user);
    let (bu, bd) = parts(configured);
    au == bu && (ad == bd || ad.is_empty() || bd.is_empty())
}

/// Audit one vCenter. `Ok(ours)` is the count attributable to the configured
/// user, excluding this audit's own session.
async fn audit(
    conn: &VCenterConnection,
    cache: &SessionCache,
    show_all: bool,
) -> Result<usize, String> {
    let session = cache.get(conn).await?;

    let sc = session.soap.service_content().await?;
    let sm = sc
        .child("sessionManager")
        .map(|e| e.text.clone())
        .unwrap_or_else(|| "SessionManager".into());

    let mo = session
        .soap
        .retrieve_moref("SessionManager", &sm, &["sessionList", "currentSession"])
        .await?
        .ok_or_else(|| "SessionManager returned no object".to_string())?;

    // Exclude the observer, so the number is not inflated by the act of asking.
    let self_key = mo
        .prop("currentSession")
        .and_then(|c| c.text_at("key"))
        .unwrap_or_default();

    let all = sessions_of(&mo);
    let now = Utc::now();

    // `lastActiveTime` is what says how close a session is to its idle
    // timeout, but vCenter does not always move it: on the 9.1.1.0 lab most
    // sessions come back with it equal to `loginTime`, which makes "idle"
    // silently identical to age. Rather than assert a rule about which builds
    // do that, measure it per run and report the proportion — a server that
    // populates the field properly is then obvious, and so is one that does
    // not. If nothing differs, idle carries no information and is suppressed.
    let stale_active = all
        .iter()
        .filter(|s| matches!(
            (s.age_minutes(now), s.idle_minutes(now)),
            (Some(age), Some(idle)) if age == idle
        ))
        .count();
    let trust_idle = !all.is_empty() && stale_active < all.len();

    let (ours, theirs): (Vec<_>, Vec<_>) = all
        .iter()
        .filter(|s| s.key != self_key)
        .partition(|s| same_user(&s.user_name, &conn.username));

    println!("\n{}", conn.label());
    println!(
        "  {} open session(s) total — {} as {}, {} other, plus this audit's own",
        all.len(),
        ours.len(),
        conn.username,
        theirs.len()
    );

    if !ours.is_empty() {
        if let Some(oldest) = ours.iter().filter_map(|s| s.age_minutes(now)).max() {
            println!("  oldest logged in {oldest} min ago");
        }
        if trust_idle {
            let idle: Vec<i64> = ours.iter().filter_map(|s| s.idle_minutes(now)).collect();
            if let (Some(min), Some(max)) = (idle.iter().min(), idle.iter().max()) {
                println!("  idle {min}–{max} min (vCenter expires at ~30)");
            }
        }
        // A pile of sessions from one address is the shape a leak makes.
        let mut by_ip: BTreeMap<&str, usize> = BTreeMap::new();
        for s in &ours {
            *by_ip.entry(s.ip_address.as_str()).or_default() += 1;
        }
        if by_ip.len() > 1 || ours.len() > 1 {
            let mut counts: Vec<_> = by_ip.into_iter().collect();
            counts.sort_by_key(|a| std::cmp::Reverse(a.1));
            let summary: Vec<String> =
                counts.iter().map(|(ip, n)| format!("{ip} ×{n}")).collect();
            println!("  from: {}", summary.join(", "));
        }
    }

    if stale_active > 0 && !all.is_empty() {
        println!(
            "  note: {stale_active}/{} session(s) report lastActiveTime == loginTime, so their \
             idle time is really just age{}",
            all.len(),
            if trust_idle { "" } else { " — idle suppressed" }
        );
    }

    if show_all {
        let mut rows: Vec<&UserSession> = all.iter().collect();
        rows.sort_by_key(|s| s.login_time);
        let mins = |v: Option<i64>| v.map(|m| m.to_string()).unwrap_or_else(|| "?".into());
        println!(
            "  {:<38} {:<32} {:<16} {:>6} {:>6}  AGENT",
            "USER", "KEY", "IP", "AGE", "IDLE"
        );
        for s in rows {
            let mark = if s.key == self_key { "*" } else { " " };
            println!(
                "{mark} {:<38} {:<32} {:<16} {:>6} {:>6}  {}",
                s.user_name,
                s.key,
                s.ip_address,
                mins(s.age_minutes(now)),
                if trust_idle { mins(s.idle_minutes(now)) } else { "—".into() },
                if s.user_agent.is_empty() { "—" } else { &s.user_agent },
            );
        }
        println!("  (* is this audit's own session, excluded from the counts above)");
    }

    Ok(ours.len())
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let show_all = args.iter().any(|a| a == "--all");
    let max: Option<usize> = args
        .iter()
        .position(|a| a == "--max")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok());

    let dir = match config::default_dir() {
        Some(d) => d,
        None => {
            eprintln!("could not determine a settings directory");
            std::process::exit(2);
        }
    };
    let cfg = match config::load(&config::config_path(dir)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("could not read the settings file: {e}");
            std::process::exit(2);
        }
    };
    let conns = match config::resolve(&cfg) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if conns.is_empty() {
        eprintln!("no vCenter is configured");
        std::process::exit(2);
    }

    let cache = SessionCache::new();
    let mut worst = 0usize;
    let mut failed = Vec::new();

    // One unreachable server must not hide the others: an inventory tool that
    // silently under-reports is worse than one that says it could not look.
    for conn in &conns {
        match audit(conn, &cache, show_all).await {
            Ok(ours) => worst = worst.max(ours),
            Err(e) => {
                eprintln!("\n{}: {e}", conn.label());
                failed.push(conn.label());
            }
        }
    }

    // The audit must not be a source of the thing it measures.
    cache.close_all().await;

    if !failed.is_empty() {
        eprintln!("\ncould not audit: {}", failed.join(", "));
    }
    if let Some(max) = max {
        if worst > max {
            eprintln!("\n{worst} session(s) of ours, over the --max of {max}");
            std::process::exit(1);
        }
    }
    if !failed.is_empty() {
        std::process::exit(2);
    }
}
