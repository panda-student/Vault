//! Isolated browser acceptance target. This mode never opens the credential
//! vault and never exports input values, only boolean acceptance evidence.
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use tauri::Manager;
struct State {
    directory: PathBuf,
    last: AtomicU64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    seq: u64,
    action: Action,
    field: Option<Field>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Focus,
    Prefix,
    Verify,
    Close,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Field {
    Account,
    Password,
    Api,
    Token,
    Ui,
    Service,
    Other,
    Readonly,
    Button,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Ack {
    seq: u64,
    matched: bool,
    focused: bool,
    submit_count: u32,
}
#[tauri::command]
fn fixture_poll(app: tauri::AppHandle, state: tauri::State<'_, State>) -> Option<Command> {
    let bytes = std::fs::read(state.directory.join("command.json")).ok()?;
    let command: Command = serde_json::from_slice(&bytes).ok()?;
    let previous = state.last.load(Ordering::SeqCst);
    if command.seq <= previous
        || state
            .last
            .compare_exchange(previous, command.seq, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
    {
        return None;
    }
    if matches!(command.action, Action::Close) {
        app.exit(0);
        return None;
    }
    if matches!(command.action, Action::Focus | Action::Prefix) {
        if let Some(window) = app.get_webview_window("fixture") {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
    Some(command)
}
#[tauri::command]
fn fixture_ack(state: tauri::State<'_, State>, ack: Ack) -> std::result::Result<(), String> {
    std::fs::write(
        state.directory.join(format!("result-{}.json", ack.seq)),
        serde_json::to_vec(&ack).map_err(|_| "Fixture encoding failed")?,
    )
    .map_err(|_| "Fixture result write failed".into())
}
pub fn run(directory: PathBuf) -> std::result::Result<(), Box<dyn std::error::Error>> {
    if !directory.is_absolute() {
        return Err("Fixture directory must be absolute".into());
    }
    std::fs::create_dir_all(&directory)?;
    tauri::Builder::default()
        .manage(State {
            directory: directory.clone(),
            last: AtomicU64::new(0),
        })
        .invoke_handler(tauri::generate_handler![fixture_poll, fixture_ack])
        .setup(move |app| {
            tauri::WebviewWindowBuilder::new(
                app,
                "fixture",
                tauri::WebviewUrl::App("fill-fixture.html".into()),
            )
            .additional_browser_args("--force-renderer-accessibility")
            .title("Credential acceptance target · synthetic fields only")
            .inner_size(680.0, 650.0)
            .data_directory(directory.join("webview"))
            .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())?;
    Ok(())
}
