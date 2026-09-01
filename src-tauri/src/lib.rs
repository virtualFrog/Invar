pub mod data;
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

/// Sheets the UI can ask for, in tab order. One list, so adding a sheet is a
/// single edit here plus its module.
#[tauri::command]
fn list_sheets() -> Vec<&'static str> {
    vec!["vInfo", "vHost"]
}

#[tauri::command]
async fn fetch_sheet(sheet: String, state: tauri::State<'_, AppState>) -> Result<Table, String> {
    let conns = state.connections()?;
    if conns.is_empty() {
        return Err("No vCenter connections configured — add one in Settings.".into());
    }
    match sheet.as_str() {
        "vInfo" => Ok(data::vinfo::fetch_vinfo_all(&conns, &state.cache).await),
        "vHost" => Ok(data::vhost::fetch_vhost_all(&conns, &state.cache).await),
        other => Err(format!("Unknown sheet: {other}")),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
            fetch_sheet
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
