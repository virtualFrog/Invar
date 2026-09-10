//! `invar-export` — the whole inventory, with no window.
//!
//! RVTools' `-c ExportAll2xls` is how people actually run RVTools: nightly, out
//! of a scheduler, into a folder someone else reads. A desktop app with an
//! export button cannot be scheduled, so this is the same export path driven
//! from a command line. It shares `config.json` with the GUI, so a vCenter
//! configured once is configured for both.
//!
//! ## Credentials
//!
//! Passwords are never in `config.json`. On a desktop they come from the OS
//! credential store; on a server, where there is none, from the environment:
//!
//! ```text
//! INVAR_PASSWORD_1=…   first connection in the list
//! INVAR_PASSWORD_2=…   second, and so on
//! INVAR_PASSWORD=…     alias for the first
//! ```
//!
//! ## Exit status
//!
//! `0` clean, `1` nothing was produced, `2` the export was written but at least
//! one vCenter contributed a warning. The third case exists because an
//! inventory that is quietly short of one server is worse than one that fails:
//! a scheduled job needs to be able to tell the difference, and a plain `0`
//! cannot.

use invar_lib::data;
use invar_lib::export;
use invar_lib::vcenter::{config, SessionCache};
use std::path::PathBuf;

const USAGE: &str = "\
invar-export — export vCenter inventory without the desktop app

USAGE:
    invar-export [OPTIONS]

OUTPUT (at least one is required):
    --xlsx <FILE>        write a multi-sheet workbook
    --csv <DIR>          write one CSV per sheet into DIR (created if absent)

OPTIONS:
    --config <FILE>      settings file [default: the desktop app's config.json]
    --sheets <A,B,C>     only these sheets, by RVTools name [default: all]
    --include-file-info  also fetch vFileInfo. Off by default: it walks
                         datastore filesystems and can take many minutes
    --list-sheets        print the sheet names and exit
    --quiet              only print warnings and errors
    -h, --help           this text

CREDENTIALS:
    Passwords come from the OS credential store, or from INVAR_PASSWORD_<n>
    (1-based, matching the connection order in config.json). A server with no
    credential store uses the environment.

EXIT STATUS:
    0  exported cleanly
    1  produced nothing
    2  exported, but at least one vCenter reported a warning
";

struct Args {
    config: Option<PathBuf>,
    xlsx: Option<PathBuf>,
    csv: Option<PathBuf>,
    sheets: Option<Vec<String>>,
    include_file_info: bool,
    quiet: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        config: None,
        xlsx: None,
        csv: None,
        sheets: None,
        include_file_info: false,
        quiet: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        // Every value-taking flag reports the flag it belongs to, so a
        // truncated command line says which argument is missing.
        let mut value = |flag: &str| -> Result<String, String> {
            it.next().ok_or_else(|| format!("{flag} needs a value"))
        };
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--list-sheets" => {
                for spec in data::SHEETS {
                    println!("{}", spec.name);
                }
                std::process::exit(0);
            }
            "--config" => args.config = Some(PathBuf::from(value("--config")?)),
            "--xlsx" => args.xlsx = Some(PathBuf::from(value("--xlsx")?)),
            "--csv" => args.csv = Some(PathBuf::from(value("--csv")?)),
            "--sheets" => {
                args.sheets = Some(
                    value("--sheets")?
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect(),
                )
            }
            "--include-file-info" => args.include_file_info = true,
            "--quiet" => args.quiet = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if args.xlsx.is_none() && args.csv.is_none() {
        return Err("nothing to write: pass --xlsx, --csv, or both".into());
    }
    Ok(args)
}

/// Resolve `--sheets` against the registry, rejecting names that do not exist.
///
/// A typo used to be indistinguishable from a sheet that legitimately had no
/// rows, which is the same silent-under-reporting failure the architecture
/// notes warn about, so an unknown name is an error rather than an empty file.
fn select_sheets(
    requested: &Option<Vec<String>>,
    include_file_info: bool,
) -> Result<Vec<&'static data::snapshot::SheetSpec>, String> {
    match requested {
        Some(names) => {
            let mut out = Vec::new();
            for name in names {
                let spec = data::SHEETS
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case(name))
                    .ok_or_else(|| {
                        format!("no such sheet: {name}. Run --list-sheets for the names.")
                    })?;
                out.push(*spec);
            }
            Ok(out)
        }
        None => Ok(data::SHEETS
            .iter()
            .copied()
            .filter(|s| include_file_info || !s.wants_files)
            .collect()),
    }
}

#[tokio::main]
async fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("invar-export: {e}\n");
            eprint!("{USAGE}");
            std::process::exit(1);
        }
    };

    match run(args).await {
        Ok(warnings) if warnings.is_empty() => std::process::exit(0),
        Ok(warnings) => {
            eprintln!("\ninvar-export: completed with {} warning(s):", warnings.len());
            for w in &warnings {
                eprintln!("  {w}");
            }
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("invar-export: {e}");
            std::process::exit(1);
        }
    }
}

async fn run(args: Args) -> Result<Vec<String>, String> {
    // Sheet names are validated before anything touches the filesystem or the
    // network, so a typo fails in a millisecond rather than after a login.
    let specs = select_sheets(&args.sheets, args.include_file_info)?;

    let path = match args.config {
        Some(p) => p,
        None => config::default_path()?,
    };
    let cfg = config::load(&path)?;
    if cfg.connections.is_empty() {
        return Err(format!(
            "no vCenter connections in {}. Configure them in the desktop app, or write the file by hand.",
            path.display()
        ));
    }
    let conns = config::resolve(&cfg)?;
    let servers: Vec<String> = conns.iter().map(|c| c.label()).collect();

    if !args.quiet {
        eprintln!(
            "invar-export: {} sheet(s) from {} vCenter(s): {}",
            specs.len(),
            conns.len(),
            servers.join(", ")
        );
    }

    let cache = SessionCache::new();
    let quiet = args.quiet;
    let tables = data::snapshot::fetch_tables_with_progress(&specs, &conns, &cache, &move |p| {
        if !quiet {
            eprintln!("  [{}/{}] {}", p.done, p.total, p.stage);
        }
    })
    .await;

    // Log out rather than leaving sessions to their ~30 minute idle timeout.
    // A job that runs every hour would otherwise hold every session it ever
    // opened, which is exactly how the reference implementation reached ~300.
    cache.close_all().await;

    if let Some(out) = &args.xlsx {
        if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        }
        export::write_workbook(&tables, &servers, out)?;
        if !args.quiet {
            println!("{}", out.display());
        }
    }

    if let Some(dir) = &args.csv {
        let files = export::write_csv_dir(&tables, &servers, dir)?;
        if !args.quiet {
            for f in &files {
                println!("{}", f.display());
            }
        }
    }

    if !args.quiet {
        let rows: usize = tables.iter().map(|t| t.rows.len()).sum();
        eprintln!("invar-export: {} row(s) across {} sheet(s)", rows, tables.len());
    }

    Ok(tables.iter().flat_map(|t| t.warnings.clone()).collect())
}
