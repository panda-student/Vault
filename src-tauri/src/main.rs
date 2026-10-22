#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod ai_output;
mod core;
mod credential_write;
mod crypto;
mod error;
mod fill;
mod fixture;
mod mcp;
mod mcp_bridge;
mod startup;
mod vault;
use core::{
    CoreSession, EntryInput, EntryPatch, EntrySummary, FieldRequest, Kind, Operation, Search,
    Settings,
};
use error::{AppError, Result};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::{
    Manager, WebviewUrl, WebviewWindowBuilder,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_clipboard_manager::ClipboardExt;
struct DesktopClipboard(tauri::AppHandle);
impl core::ClipboardPort for DesktopClipboard {
    fn write_text(&self, text: String) -> Result<()> {
        self.0
            .clipboard()
            .write_text(text)
            .map_err(|_| AppError::new("CLIPBOARD_ERROR", "无法写入系统剪贴板，请重试。"))
    }
}
struct AppState {
    core: Arc<CoreSession>,
    executable: PathBuf,
    smoke_report: Option<PathBuf>,
    smoke_finished: AtomicBool,
    update_health: Option<(PathBuf, String)>,
}
#[tauri::command]
async fn update_action(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    action: String,
) -> Result<serde_json::Value> {
    let operation = match action.as_str() {
        "status" => Operation::UpdateStatus,
        "check" => Operation::CheckUpdate,
        "download" => Operation::DownloadUpdate,
        "cancel" => Operation::CancelUpdate,
        "install" => Operation::InstallUpdate,
        _ => return Err(AppError::invalid()),
    };
    let core = state.core.clone();
    let result = tauri::async_runtime::spawn_blocking(move || core.execute(operation))
        .await
        .map_err(|_| AppError::new("UPDATE_ERROR", "更新任务未完成，请重试。"))??;
    if action == "install" {
        app.exit(0);
    }
    Ok(result)
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopInfo {
    version: &'static str,
    executable: PathBuf,
    data_directory: PathBuf,
    webview_version: String,
    smoke_test: bool,
    settings: Settings,
    startup_enabled: bool,
}
#[tauri::command]
fn desktop_info(state: tauri::State<'_, AppState>) -> Result<DesktopInfo> {
    Ok(DesktopInfo {
        version: env!("CARGO_PKG_VERSION"),
        executable: state.executable.clone(),
        data_directory: state.core.directory().to_owned(),
        webview_version: tauri::webview_version()
            .map_err(|_| AppError::new("WEBVIEW_ERROR", "无法读取 WebView2 版本。"))?,
        smoke_test: state.smoke_report.is_some(),
        settings: state.core.call(Operation::Settings)?,
        startup_enabled: state
            .core
            .call(Operation::StartupEnabled(state.executable.clone()))?,
    })
}
#[tauri::command]
fn list_entries(
    state: tauri::State<'_, AppState>,
    query: Option<String>,
    kind: Option<Kind>,
) -> Result<Vec<EntrySummary>> {
    let result = state.core.execute(Operation::Search(Search::new(
        query.unwrap_or_default(),
        kind,
    )))?;
    serde_json::from_value(result["entries"].clone()).map_err(|_| AppError::storage())
}
#[tauri::command]
fn create_entry(state: tauri::State<'_, AppState>, input: EntryInput) -> Result<EntrySummary> {
    Ok(state
        .core
        .call::<Vec<EntrySummary>>(Operation::Create(vec![input]))?
        .remove(0))
}
#[tauri::command]
fn update_entry(state: tauri::State<'_, AppState>, patch: EntryPatch) -> Result<EntrySummary> {
    Ok(state
        .core
        .call::<Vec<EntrySummary>>(Operation::Update(vec![patch]))?
        .remove(0))
}
#[tauri::command]
fn pin_entry(
    state: tauri::State<'_, AppState>,
    id: String,
    expected_revision: i64,
    pinned: bool,
) -> Result<EntrySummary> {
    state.core.call(Operation::Pin {
        id,
        expected_revision,
        pinned,
    })
}
#[tauri::command]
fn delete_entry(
    state: tauri::State<'_, AppState>,
    id: String,
    expected_revision: i64,
) -> Result<()> {
    state
        .core
        .execute(Operation::Delete(vec![core::EntryVersion {
            id,
            expected_revision,
        }]))?;
    Ok(())
}
#[tauri::command]
fn get_organization(state: tauri::State<'_, AppState>) -> Result<core::Organization> {
    state.core.call(Operation::Organization)
}
#[tauri::command]
fn organize(
    state: tauri::State<'_, AppState>,
    kind: String,
    name: String,
    previous: Option<String>,
) -> Result<()> {
    state.core.call(Operation::Organize {
        kind,
        name,
        previous,
    })
}
#[tauri::command]
fn list_trash(state: tauri::State<'_, AppState>) -> Result<Vec<core::TrashEntry>> {
    state.core.call(Operation::ListTrash)
}
#[tauri::command]
fn restore_entry(
    state: tauri::State<'_, AppState>,
    id: String,
    expected_revision: i64,
) -> Result<EntrySummary> {
    state.core.call(Operation::Restore {
        id,
        expected_revision,
    })
}
#[tauri::command]
fn empty_trash(window: tauri::WebviewWindow, state: tauri::State<'_, AppState>) -> Result<usize> {
    let owner_window = window.hwnd().map_err(|_| AppError::storage())?.0 as isize;
    state.core.call(Operation::EmptyTrash { owner_window })
}
#[tauri::command]
fn read_secret(
    state: tauri::State<'_, AppState>,
    id: String,
    expected_revision: i64,
) -> Result<String> {
    state.core.call(Operation::ReadField(FieldRequest {
        id,
        expected_revision,
        field: "secret".into(),
        service_id: None,
    }))
}
#[tauri::command]
fn copy_field(
    state: tauri::State<'_, AppState>,
    id: String,
    expected_revision: i64,
    field: String,
    service_id: Option<String>,
) -> Result<()> {
    state.core.call(Operation::CopyField(FieldRequest {
        id,
        expected_revision,
        field,
        service_id,
    }))
}
#[tauri::command]
fn copy_text(state: tauri::State<'_, AppState>, text: String) -> Result<()> {
    state.core.call(Operation::CopyText(text))
}
#[tauri::command]
fn save_settings(state: tauri::State<'_, AppState>, settings: Settings) -> Result<Settings> {
    state.core.call(Operation::SaveSettings(settings))
}
#[tauri::command]
fn permission_preview(
    state: tauri::State<'_, AppState>,
    settings: Settings,
) -> Result<serde_json::Value> {
    state.core.execute(Operation::PermissionPreview(settings))
}
#[tauri::command]
fn set_startup(state: tauri::State<'_, AppState>, enabled: bool) -> Result<bool> {
    state.core.call(Operation::SetStartup {
        executable: state.executable.clone(),
        enabled,
    })
}
#[tauri::command]
fn smoke_clipboard(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Option<bool>> {
    if state.smoke_report.is_none() {
        return Err(AppError::invalid());
    }
    // Keep any existing text entirely in memory, never in test artifacts. If
    // the clipboard holds an unsupported format, leave it untouched.
    let original = match app.clipboard().read_text() {
        Ok(value) => Some(value),
        Err(_) if clipboard_is_empty() => None,
        Err(_) => return Ok(None),
    };
    let entry = state
        .core
        .call::<Vec<EntrySummary>>(Operation::Read(vec![id.clone()]))?
        .remove(0);
    let secret: String = state.core.call(Operation::ReadField(FieldRequest {
        id: id.clone(),
        expected_revision: entry.revision,
        field: "secret".into(),
        service_id: None,
    }))?;
    let written = state.core.execute(Operation::CopyField(FieldRequest {
        id,
        expected_revision: entry.revision,
        field: "secret".into(),
        service_id: None,
    }));
    let matches = written.is_ok()
        && app
            .clipboard()
            .read_text()
            .is_ok_and(|value| value == secret);
    let restored = match original {
        Some(value) => app.clipboard().write_text(value),
        None => app.clipboard().clear(),
    };
    restored.map_err(|_| AppError::new("CLIPBOARD_ERROR", "剪贴板恢复失败。"))?;
    Ok(Some(matches))
}

fn clipboard_is_empty() -> bool {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(window: *mut std::ffi::c_void) -> i32;
        fn CountClipboardFormats() -> u32;
        fn CloseClipboard() -> i32;
    }
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return false;
        }
        let empty = CountClipboardFormats() == 0;
        CloseClipboard();
        empty
    }
}

#[tauri::command]
async fn smoke_window_cycle(app: tauri::AppHandle) -> Result<bool> {
    if app.state::<AppState>().smoke_report.is_none() {
        return Err(AppError::invalid());
    }
    let window = app.get_webview_window("main").ok_or(AppError::invalid())?;
    window.close().map_err(|_| AppError::invalid())?;
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    let hidden = !window.is_visible().map_err(|_| AppError::invalid())?;
    window.show().map_err(|_| AppError::invalid())?;
    Ok(hidden
        && window.is_visible().map_err(|_| AppError::invalid())?
        && app.tray_by_id("main").is_some())
}

#[tauri::command]
fn smoke_resize(app: tauri::AppHandle, width: u32, height: u32) -> Result<()> {
    if app.state::<AppState>().smoke_report.is_none()
        || !matches!((width, height), (1120, 740) | (900, 580))
    {
        return Err(AppError::invalid());
    }
    app.get_webview_window("main")
        .ok_or(AppError::invalid())?
        .set_size(tauri::LogicalSize::new(width, height))
        .map_err(|_| AppError::invalid())
}

#[tauri::command]
fn finish_smoke(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    report: serde_json::Value,
) -> Result<()> {
    let destination = state.smoke_report.as_ref().ok_or(AppError::invalid())?;
    std::fs::write(
        destination,
        serde_json::to_vec_pretty(&report).map_err(|_| AppError::invalid())?,
    )
    .map_err(|_| AppError::storage())?;
    state.smoke_finished.store(true, Ordering::SeqCst);
    app.exit(
        if report.get("passed").and_then(|v| v.as_bool()) == Some(true) {
            0
        } else {
            1
        },
    );
    Ok(())
}
fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
fn desktop(
    state: AppState,
    background: bool,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    if tauri::webview_version().is_err() {
        return Err("需要 Microsoft Edge WebView2 Runtime，请从 https://developer.microsoft.com/microsoft-edge/webview2/ 获取。".into());
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            desktop_info,
            list_entries,
            create_entry,
            update_entry,
            pin_entry,
            delete_entry,
            list_trash,
            get_organization,
            organize,
            restore_entry,
            empty_trash,
            read_secret,
            copy_field,
            copy_text,
            save_settings,
            permission_preview,
            set_startup,
            update_action,
            smoke_clipboard,
            smoke_window_cycle,
            smoke_resize,
            finish_smoke
        ])
        .setup(move |app| {
            let state = app.state::<AppState>();
            state
                .core
                .bind_clipboard(Arc::new(DesktopClipboard(app.handle().clone())))?;
            let is_smoke = state.smoke_report.is_some();
            let builder =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("本地密码库")
                    .inner_size(1120.0, 740.0)
                    .min_inner_size(900.0, 580.0)
                    .focused(!is_smoke)
                    .visible(!background)
                    .data_directory(state.core.directory().join("webview"));
            // A debug build can expose only its explicitly selected fixture window
            // for native UI inspection. Release builds do not include this hook.
            #[cfg(debug_assertions)]
            let builder = if let Ok(port) = std::env::var("VAULT_UI_DEBUG_PORT") {
                let port = port.parse::<u16>().map_err(|_| "Invalid UI debug port")?;
                builder.additional_browser_args(&format!("--remote-debugging-port={port}"))
            } else {
                builder
            };
            let window = builder.build()?;
            let observed = window.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event
                    && observed
                        .state::<AppState>()
                        .core
                        .call::<Settings>(Operation::Settings)
                        .is_ok_and(|s| s.close_to_tray)
                {
                    api.prevent_close();
                    let _ = observed.hide();
                }
            });
            let open = MenuItem::with_id(app, "open", "打开本地密码库", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            let mut tray = TrayIconBuilder::with_id("main")
                .tooltip("本地密码库")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        show_main(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            if let Some((path, token)) = &state.update_health {
                std::fs::write(path, token)?;
            }
            if is_smoke {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                    let state = handle.state::<AppState>();
                    if !state.smoke_finished.load(Ordering::SeqCst) {
                        if let Some(path) = &state.smoke_report {
                            let _ = std::fs::write(
                                path,
                                r#"{"passed":false,"error":"Desktop smoke test timed out"}"#,
                            );
                        }
                        handle.exit(1);
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())?;
    Ok(())
}
fn show_error(message: &str) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(
            window: *mut std::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            flags: u32,
        ) -> i32;
    }
    let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "本地密码库".encode_utf16().chain(Some(0)).collect();
    unsafe {
        MessageBoxW(std::ptr::null_mut(), message.as_ptr(), title.as_ptr(), 0x10);
    }
}
fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let bootstrap_args: Vec<_> = std::env::args_os().skip(1).collect();
    if bootstrap_args.first().is_some_and(|arg|arg=="--installer-replace") {
        if bootstrap_args.len()!=3{return Err("Invalid installer arguments".into());}
        return core::replace_package(&PathBuf::from(&bootstrap_args[1]),&PathBuf::from(&bootstrap_args[2])).map_err(Into::into);
    }
    if bootstrap_args
        .first()
        .is_some_and(|arg| arg == "--apply-update")
    {
        if bootstrap_args.len() != 2 {
            return Err("Invalid updater arguments".into());
        }
        return core::apply_update(&PathBuf::from(&bootstrap_args[1])).map_err(Into::into);
    }
    let mut is_mcp = false;
    let mut mcp_service = false;
    let mut directory = None;
    let mut report = None;
    let mut background = false;
    let mut fixture_directory = None;
    let mut update_health = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--update-health") => {
                let path = PathBuf::from(args.next().ok_or("Missing health path")?);
                let token = args
                    .next()
                    .ok_or("Missing health token")?
                    .into_string()
                    .map_err(|_| "Invalid health token")?;
                update_health = Some((path, token));
            }
            Some("--mcp") => is_mcp = true,
            Some("--mcp-service") => { is_mcp = true; mcp_service = true; }
            Some("--data-dir") => {
                directory = Some(PathBuf::from(
                    args.next().ok_or("--data-dir requires a path")?,
                ))
            }
            Some("--smoke-test") => {
                report = Some(PathBuf::from(
                    args.next().ok_or("--smoke-test requires a path")?,
                ))
            }
            Some("--background") => background = true,
            Some("--fill-fixture") => {
                fixture_directory = Some(PathBuf::from(
                    args.next().ok_or("--fill-fixture requires a directory")?,
                ))
            }
            _ => return Err("Unknown argument".into()),
        }
    }
    if let Some(path) = fixture_directory {
        if is_mcp || report.is_some() || directory.is_some() || background {
            return Err("Fixture mode cannot be combined with other modes".into());
        }
        return fixture::run(path);
    }
    if report.is_some() && directory.is_none() {
        return Err("--smoke-test requires an explicit --data-dir".into());
    }
    let executable = std::env::current_exe()?;
    if let Some((path, token)) = &update_health {
        let stage = path.parent().ok_or("Invalid health path")?;
        if is_mcp
            || !path.is_absolute()
            || path.file_name().and_then(|n| n.to_str()) != Some("healthy")
            || stage.parent().and_then(|p|std::fs::canonicalize(p).ok())
                != executable.parent().and_then(|p|std::fs::canonicalize(p).ok())
            || !stage
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(".vault-update-"))
            || uuid::Uuid::parse_str(token).is_err()
        {
            return Err("Invalid update health arguments".into());
        }
    }
    let directory = directory.unwrap_or(
        executable
            .parent()
            .ok_or("No executable directory")?
            .join("data"),
    );
    if !directory.is_absolute() {
        return Err("--data-dir must be absolute".into());
    }
    if report.as_ref().is_some_and(|p| !p.is_absolute()) {
        return Err("--smoke-test requires an absolute report path".into());
    }
    if is_mcp && (report.is_some() || background) {
        return Err("MCP cannot be combined with desktop arguments".into());
    }
    if is_mcp && !mcp_service {
        return mcp_bridge::run(&executable, &directory);
    }
    if mcp_service {
        return mcp_bridge::service::run(&executable, &directory);
    }
    let runtime = core::UpdateRuntime::acquire(&executable)?;
    let core = core::open(&directory, is_mcp)?;
    core.bind_update_runtime(runtime)?;
    desktop(
        AppState {
            core,
            executable,
            smoke_report: report,
            smoke_finished: AtomicBool::new(false),
            update_health,
        },
        background,
    )
}
fn main() {
    let installer = std::env::args_os().nth(1).is_some_and(|arg|arg=="--installer-replace");
    let health_probe = std::env::args_os().any(|arg|arg=="--update-health");
    let is_mcp = std::env::args_os().any(|arg| arg == "--mcp" || arg == "--mcp-service");
    if is_mcp {
        std::panic::set_hook(Box::new(|_| eprintln!("Local credential service failed")));
    }
    if let Err(error) = run() {
        if is_mcp {
            eprintln!("Local credential service could not complete startup");
        } else {
            eprintln!("vault: {error}");
        }
        if !is_mcp && !installer && !health_probe {
            show_error(&error.to_string());
        }
        std::process::exit(1);
    }
}
