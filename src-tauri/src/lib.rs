pub mod data;
pub mod export;
pub mod report;
pub mod vcenter;

use data::Table;
use std::sync::Arc;
use tauri::Manager;
use vcenter::config::{self, AppConfig};
use vcenter::{SessionCache, VCenterConnection};

pub struct AppState {
    pub cache: Arc<SessionCache>,
    pub config_path: std::path::PathBuf,
}

impl AppState {
    fn connections(&self) -> Result<Vec<VCenterConnection>, String> {
        Ok(config::load(&self.config_path)?.connections)
    }
}

// ---- Tauri commands: thin wrappers over the framework-free core ----

#[tauri::command]
async fn get_config(state: tauri::State<'_, AppState>) -> Result<AppConfig, String> {
    config::load(&state.config_path)
}

#[tauri::command]
async fn save_config(cfg: AppConfig, state: tauri::State<'_, AppState>) -> Result<(), String> {
    config::save(&state.config_path, &cfg)
}

#[tauri::command]
async fn test_connection(conn: VCenterConnection, state: tauri::State<'_, AppState>) -> Result<String, String> {
    let session = state.cache.get(&conn).await?;
    let about = session
        .soap
        .call(r#"<vim25:RetrieveServiceContent><vim25:_this type="ServiceInstance">ServiceInstance</vim25:_this></vim25:RetrieveServiceContent>"#)
        .await?;
    let full_name = about
        .find("about")
        .and_then(|a| a.text_at("fullName"))
        .unwrap_or_else(|| "connected".into());
    Ok(full_name)
}

/// Sheets the UI can ask for, in tab order. Driven off `data::SHEETS`, so
/// adding a sheet needs no edit here.
#[tauri::command]
fn list_sheets() -> Vec<&'static str> {
    data::SHEETS.iter().map(|s| s.name).collect()
}

#[tauri::command]
async fn fetch_sheet(sheet: String, state: tauri::State<'_, AppState>) -> Result<Table, String> {
    let conns = state.connections()?;
    if conns.is_empty() {
        return Err("No vCenter connections configured — add one in Settings.".into());
    }
    let spec = data::SHEETS
        .iter()
        .find(|s| s.name == sheet)
        .ok_or_else(|| format!("Unknown sheet: {sheet}"))?;
    // One sheet fetches only the properties that sheet reads, so opening a tab
    // does not pay for the whole export's property union.
    Ok(data::snapshot::fetch_table(spec, &conns, &state.cache).await)
}

/// Fetch every sheet the app knows about, for the export.
///
/// Returns the tables plus the servers they came from, and never fails as a
/// whole: a sheet that errors contributes its warning and an empty table, so an
/// export is never silently short of a sheet without saying so.
///
/// Every sheet is built from one snapshot per vCenter, so the cost is one
/// inventory walk per object type rather than one per sheet.
async fn fetch_all_tables(state: &AppState) -> Result<(Vec<Table>, Vec<String>), String> {
    let conns = state.connections()?;
    if conns.is_empty() {
        return Err("No vCenter connections configured — add one in Settings.".into());
    }
    let servers = conns.iter().map(|c| c.label()).collect();
    // One inventory fetch per vCenter, shared by every sheet, rather than one
    // walk per sheet.
    let tables = data::snapshot::fetch_tables(data::SHEETS, &conns, &state.cache).await;
    Ok((tables, servers))
}

/// What an export produced, for the UI to report.
#[derive(serde::Serialize)]
struct ExportResult {
    /// `None` when the user dismissed the save dialog.
    path: Option<String>,
    sheets: usize,
    rows: usize,
    /// Per-vCenter failures. An export that is missing a server's data says so.
    warnings: Vec<String>,
}

#[tauri::command]
async fn fetch_insights(
    state: tauri::State<'_, AppState>,
) -> Result<data::insights::Insights, String> {
    let conns = state.connections()?;
    if conns.is_empty() {
        return Err("No vCenter connections configured — add one in Settings.".into());
    }
    Ok(data::insights::fetch_insights_all(&conns, &state.cache).await)
}

#[tauri::command]
async fn export_xlsx(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<ExportResult, String> {
    use tauri_plugin_dialog::DialogExt;

    let (tables, servers) = fetch_all_tables(&state).await?;

    let dialog = app
        .dialog()
        .file()
        .set_title("Export inventory")
        .set_file_name(export::default_filename())
        .add_filter("Excel workbook", &["xlsx"]);
    // The dialog blocks until the user answers, so it must not run on the
    // async runtime's thread.
    let chosen = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .map_err(|e| format!("save dialog failed: {e}"))?;

    let Some(path) = chosen else {
        return Ok(ExportResult { path: None, sheets: 0, rows: 0, warnings: Vec::new() });
    };
    let path = path
        .into_path()
        .map_err(|e| format!("could not resolve the chosen path: {e}"))?;

    let rows = tables.iter().map(|t| t.rows.len()).sum();
    let warnings = tables.iter().flat_map(|t| t.warnings.clone()).collect();
    let sheets = tables.len();

    export::write_workbook(&tables, &servers, &path)?;

    Ok(ExportResult {
        path: Some(path.display().to_string()),
        sheets,
        rows,
        warnings,
    })
}

/// What the topology report covered, for the UI to report.
#[derive(serde::Serialize)]
struct ReportResult {
    /// `None` when the user dismissed the save dialog.
    path: Option<String>,
    hosts: usize,
    datastores: usize,
    warnings: Vec<String>,
}

#[tauri::command]
async fn export_topology_report(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<ReportResult, String> {
    use tauri_plugin_dialog::DialogExt;

    let conns = state.connections()?;
    if conns.is_empty() {
        return Err("No vCenter connections configured — add one in Settings.".into());
    }
    let topology = data::topology::fetch_topology_all(&conns, &state.cache).await;

    let dialog = app
        .dialog()
        .file()
        .set_title("Save topology report")
        .set_file_name(report::default_filename())
        .add_filter("HTML report", &["html"]);
    let chosen = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .map_err(|e| format!("save dialog failed: {e}"))?;

    let Some(path) = chosen else {
        return Ok(ReportResult { path: None, hosts: 0, datastores: 0, warnings: Vec::new() });
    };
    let path = path
        .into_path()
        .map_err(|e| format!("could not resolve the chosen path: {e}"))?;

    let hosts = topology.servers.iter().map(|s| s.all_hosts().len()).sum();
    let datastores = topology.servers.iter().map(|s| s.datastores.len()).sum();

    std::fs::write(&path, report::render(&topology))
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;

    Ok(ReportResult {
        path: Some(path.display().to_string()),
        hosts,
        datastores,
        warnings: topology.warnings,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_config_dir().map_err(|e| format!("no config dir: {e}"))?;
            app.manage(AppState {
                cache: Arc::new(SessionCache::new()),
                config_path: config::config_path(dir),
            });

            // vCenter sessions linger for ~30 minutes after the app exits unless
            // they are closed explicitly. SIGTERM matters as much as SIGINT:
            // a service restart sends TERM, and Ctrl-C-only handling leaks on
            // every one.
            let cache = Arc::clone(&app.state::<AppState>().cache);
            spawn_shutdown_handler(cache);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            test_connection,
            list_sheets,
            fetch_sheet,
            fetch_insights,
            export_xlsx,
            export_topology_report
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let cache = Arc::clone(&app_handle.state::<AppState>().cache);
                tauri::async_runtime::block_on(cache.close_all());
            }
        });
}

#[cfg(unix)]
fn spawn_shutdown_handler(cache: Arc<SessionCache>) {
    tauri::async_runtime::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let (mut int, mut term) = match (signal(SignalKind::interrupt()), signal(SignalKind::terminate())) {
            (Ok(i), Ok(t)) => (i, t),
            _ => {
                eprintln!("warning: could not install signal handlers; sessions may leak on exit");
                return;
            }
        };
        tokio::select! {
            _ = int.recv() => {}
            _ = term.recv() => {}
        }
        cache.close_all().await;
        std::process::exit(0);
    });
}

#[cfg(not(unix))]
fn spawn_shutdown_handler(cache: Arc<SessionCache>) {
    tauri::async_runtime::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            cache.close_all().await;
            std::process::exit(0);
        }
    });
}
