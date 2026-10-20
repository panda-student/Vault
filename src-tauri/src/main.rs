#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod ai_output;
mod credential_write;
mod crypto;
mod error;
mod fill;
mod fixture;
mod mcp;
mod startup;
mod vault;
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
use vault::{EntryInput, EntryPatch, EntrySummary, Kind, Settings, Vault};
struct AppState {
    vault: Arc<Vault>,
    executable: PathBuf,
    smoke_report: Option<PathBuf>,
    smoke_finished: AtomicBool,
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
        data_directory: state.vault.directory().to_owned(),
        webview_version: tauri::webview_version()
            .map_err(|_| AppError::new("WEBVIEW_ERROR", "无法读取 WebView2 版本。"))?,
        smoke_test: state.smoke_report.is_some(),
        settings: state.vault.settings()?,
        startup_enabled: startup::enabled(&state.executable, state.vault.directory())?,
    })
}
#[tauri::command]
fn list_entries(
    state: tauri::State<'_, AppState>,
    query: Option<String>,
    kind: Option<Kind>,
) -> Result<Vec<EntrySummary>> {
    state.vault.list(query.as_deref().unwrap_or(""), kind)
}
#[tauri::command]
fn create_entry(state: tauri::State<'_, AppState>, input: EntryInput) -> Result<EntrySummary> {
    state.vault.create(input)
}
#[tauri::command]
fn update_entry(state: tauri::State<'_, AppState>, patch: EntryPatch) -> Result<EntrySummary> {
    state.vault.update(patch)
}
#[tauri::command]
fn pin_entry(
    state: tauri::State<'_, AppState>,
    id: String,
    expected_revision: i64,
    pinned: bool,
) -> Result<EntrySummary> {
    state.vault.pin(&id, expected_revision, pinned)
}
#[tauri::command]
fn delete_entry(
    state: tauri::State<'_, AppState>,
    id: String,
    expected_revision: i64,
) -> Result<()> {
    state.vault.delete(&id, expected_revision)
}
#[tauri::command]
fn get_organization(state: tauri::State<'_, AppState>) -> Result<vault::Organization> {
    state.vault.organization()
}
#[tauri::command]
fn organize(
    state: tauri::State<'_, AppState>,
    kind: String,
    name: String,
    previous: Option<String>,
) -> Result<()> {
    state.vault.organize(&kind, &name, previous.as_deref())
}
#[tauri::command]
fn list_trash(state: tauri::State<'_, AppState>) -> Result<Vec<vault::TrashEntry>> {
    state.vault.list_trash()
}
#[tauri::command]
fn restore_entry(
    state: tauri::State<'_, AppState>,
    id: String,
    expected_revision: i64,
) -> Result<EntrySummary> {
    state.vault.restore(&id, expected_revision)
}
#[tauri::command]
fn empty_trash(window: tauri::WebviewWindow, state: tauri::State<'_, AppState>) -> Result<usize> {
    use windows::{
        Win32::UI::WindowsAndMessaging::{
            IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_YESNO, MessageBoxW,
        },
        core::PCWSTR,
    };
    let items = state.vault.list_trash()?;
    if items.is_empty() {
        return Ok(0);
    }
    let text: Vec<u16> = format!(
        "将永久删除垃圾桶中的全部 {} 条凭据。此操作无法恢复。是否继续？",
        items.len()
    )
    .encode_utf16()
    .chain(Some(0))
    .collect();
    let title: Vec<u16> = "确认永久清空垃圾桶".encode_utf16().chain(Some(0)).collect();
    let hwnd = window.hwnd().map_err(|_| AppError::storage())?;
    // Native human confirmation; no caller-provided boolean can bypass this dialog.
    let answer = unsafe {
        MessageBoxW(
            Some(windows::Win32::Foundation::HWND(hwnd.0)),
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
        )
    };
    if answer != IDYES {
        return Ok(0);
    }
    state.vault.empty_trash(
        items
            .into_iter()
            .map(|e| vault::EntryVersion {
                id: e.summary.id,
                expected_revision: e.summary.revision,
            })
            .collect(),
    )
}
#[tauri::command]
fn read_secret(
    state: tauri::State<'_, AppState>,
    id: String,
    expected_revision: i64,
) -> Result<String> {
    let detail = state.vault.get(&id)?;
    if detail.summary.revision != expected_revision {
        return Err(AppError::conflict());
    }
    Ok(detail.secret)
}
#[tauri::command]
fn copy_field(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
    expected_revision: i64,
    field: String,
    service_id: Option<String>,
) -> Result<()> {
    let entry = state.vault.get(&id)?;
    if entry.summary.revision != expected_revision {
        return Err(AppError::conflict());
    }
    let value = match field.as_str() {
        "account" => entry.summary.account,
        "url" if service_id.is_some() => entry.summary.service(service_id.as_deref())?.url.clone(),
        "url" => entry.summary.url,
        "secret" => entry.secret,
        _ => return Err(AppError::invalid()),
    };
    app.clipboard()
        .write_text(value)
        .map_err(|_| AppError::new("CLIPBOARD_ERROR", "无法写入系统剪贴板，请重试。"))
}
#[tauri::command]
fn copy_text(app: tauri::AppHandle, text: String) -> Result<()> {
    if text.len() > 65536 {
        return Err(AppError::invalid());
    }
    app.clipboard()
        .write_text(text)
        .map_err(|_| AppError::new("CLIPBOARD_ERROR", "无法写入系统剪贴板。"))
}
#[tauri::command]
fn save_settings(state: tauri::State<'_, AppState>, settings: Settings) -> Result<Settings> {
    state.vault.save_settings(&settings)?;
    Ok(settings)
}
#[tauri::command]
fn set_startup(state: tauri::State<'_, AppState>, enabled: bool) -> Result<bool> {
    startup::set(enabled, &state.executable, state.vault.directory())?;
    startup::enabled(&state.executable, state.vault.directory())
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
    let secret = state.vault.get(&id)?.secret;
    let written = app.clipboard().write_text(secret.clone());
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
            set_startup,
            smoke_clipboard,
            smoke_window_cycle,
            smoke_resize,
            finish_smoke
        ])
        .setup(move |app| {
            let state = app.state::<AppState>();
            let is_smoke = state.smoke_report.is_some();
            let builder =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("本地密码库")
                    .inner_size(1120.0, 740.0)
                    .min_inner_size(900.0, 580.0)
                    .focused(!is_smoke)
                    .visible(!background)
                    .data_directory(state.vault.directory().join("webview"));
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
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    if observed
                        .state::<AppState>()
                        .vault
                        .settings()
                        .is_ok_and(|s| s.close_to_tray)
                    {
                        api.prevent_close();
                        let _ = observed.hide();
                    }
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
    let mut is_mcp = false;
    let mut directory = None;
    let mut report = None;
    let mut background = false;
    let mut fixture_directory = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--mcp") => is_mcp = true,
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
    let vault = Arc::new(Vault::open(&directory)?);
    if is_mcp {
        return mcp::run(vault);
    }
    desktop(
        AppState {
            vault,
            executable,
            smoke_report: report,
            smoke_finished: AtomicBool::new(false),
        },
        background,
    )
}
fn main() {
    let is_mcp = std::env::args_os().any(|arg| arg == "--mcp");
    if is_mcp {
        std::panic::set_hook(Box::new(|_| eprintln!("Local credential service failed")));
    }
    if let Err(error) = run() {
        if is_mcp {
            eprintln!("Local credential service could not complete startup");
        } else {
            eprintln!("Local Password Manager: {error}");
        }
        if !std::env::args_os().any(|a| a == "--mcp") {
            show_error(&error.to_string());
        }
        std::process::exit(1);
    }
}
