//! Unified operation interface. Adapters hold a bound session, never raw services.
//! Only bootstrap selects caller identity; neither Operation nor caller is decoded
//! from a client-provided identity field. Policy runs before every business action.
pub use filling::WriteArgs;
pub use crate::fill::CaptureRequest;
pub use crate::vault::{
    EntryInput, EntryPatch, EntrySummary, EntryVersion, Kind, Organization, Settings, TrashEntry,
};
use crate::{
    ai_output::{OutputGuard, SharedGuard},
    error::{AppError, Result},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, RwLock},
};
mod interface;
pub use interface::{ClipboardPort, FieldRequest, Operation, Search};
mod ai;
mod filling;
mod permissions;
mod storage;
#[cfg(test)]
mod tests;
mod updater;
mod windows;
mod installer;
pub use installer::replace_package;
pub use updater::{UpdateRuntime, apply_update};

struct CoreService {
    updater: updater::UpdateService,
    storage: storage::StorageService,
    filling: filling::FillingService,
    windows: windows::WindowsService,
    permissions: permissions::PermissionService,
}
pub struct CoreSession {
    execution_gate: Arc<RwLock<()>>,
    service: Arc<CoreService>,
    caller: permissions::Caller,
    guard: SharedGuard,
}
/// Bootstrap-only binding. Client payloads never reach this function.
pub fn open(directory: &Path, mcp_mode: bool) -> Result<Arc<CoreSession>> {
    Ok(Arc::new(CoreSession {
        execution_gate: Arc::new(RwLock::new(())),
        service: Arc::new(CoreService {
            updater: Default::default(),
            storage: storage::StorageService::open(directory)?,
            filling: Default::default(),
            windows: Default::default(),
            permissions: permissions::PermissionService,
        }),
        caller: if mcp_mode {
            permissions::Caller::Mcp
        } else {
            permissions::Caller::Desktop
        },
        guard: Arc::new(RwLock::new(OutputGuard::default())),
    }))
}
fn encoded(value: impl Serialize) -> Result<Value> {
    serde_json::to_value(value).map_err(|_| AppError::storage())
}
/// MCP organize must never relabel entries into more permissions than they had.
/// The simulation mirrors vault::organize: the tag's permission rule moves with a
/// rename and disappears on delete, for live entries and trash alike.
fn guard_organize(
    storage: &storage::StorageService,
    name: &str,
    previous: Option<&str>,
) -> Result<()> {
    use permissions::Access;
    let Some(old) = previous else {
        return Ok(());
    };
    let name = name.trim();
    let renaming = !name.is_empty();
    let settings = storage.settings()?;
    let actions = [
        Access::Read,
        Access::Create,
        Access::Update,
        Access::Delete,
        Access::Input,
        Access::File,
    ];
    // 模拟只允许作用于「改后」一侧；「改前」必须按原始规则评估，
    // 否则删除 deny 规则带来的权限提升在比较中不可见。
    let mut simulated = settings.clone();
    if renaming {
        if let Some(rule) = simulated.mcp_tag_rules.remove(old) {
            simulated.mcp_tag_rules.insert(name.to_string(), rule);
        }
    } else {
        simulated.mcp_tag_rules.remove(old);
    }
    let trash = storage.list_trash()?;
    for entry in storage
        .list("", None)?
        .iter()
        .chain(trash.iter().map(|t| &t.summary))
    {
        if !entry.tags.iter().any(|t| t == old) {
            continue;
        }
        let mut renamed_tags: Vec<String> =
            entry.tags.iter().filter(|t| *t != old).cloned().collect();
        if renaming && !renamed_tags.iter().any(|t| t == name) {
            renamed_tags.push(name.to_string());
        }
        for action in actions {
            if permissions::permitted(&simulated, Some(&entry.id), &renamed_tags, action)
                && !permissions::permitted(&settings, Some(&entry.id), &entry.tags, action)
            {
                return Err(AppError::new(
                    "PERMISSION_DENIED",
                    "此操作会提升 MCP 对部分凭据的权限，已被拒绝。请在桌面端完成该重命名。",
                ));
            }
        }
    }
    Ok(())
}
impl CoreSession {
    /// Bootstrap may only downgrade a desktop-bound session to MCP identity.
    /// Both identities share services, update gate, target state and output guard.
    pub fn mcp_session(&self) -> Arc<Self> {
        Arc::new(Self {
            execution_gate: self.execution_gate.clone(),
            service: self.service.clone(),
            caller: permissions::Caller::Mcp,
            guard: self.guard.clone(),
        })
    }

    /// Bootstrap only; all process modes must acquire the lease before opening storage.
    pub fn bind_update_runtime(&self, runtime: Arc<UpdateRuntime>) -> Result<()> {
        self.service.updater.bind(runtime)
    }
    /// Bootstrap capability binding, not an operation available to client payloads.
    pub fn bind_clipboard(&self, port: Arc<dyn ClipboardPort>) -> Result<()> {
        self.service.permissions.authorize_operation(
            self.caller,
            &Operation::CopyText(String::new()),
            &self.service.storage.settings()?,
            &self.service.storage,
        )?;
        self.service.windows.bind_clipboard(port)
    }
    pub fn directory(&self) -> &Path {
        self.service.storage.directory()
    }
    pub fn output_guard(&self) -> SharedGuard {
        self.guard.clone()
    }
    pub fn refresh_output_guard(&self, guard: &mut OutputGuard) -> Result<()> {
        for value in self.service.storage.protected_values()?.iter() {
            guard.add(value);
        }
        Ok(())
    }
    pub fn protect_input(&self, value: &Value) -> Result<()> {
        let mut guard = self.guard.write().map_err(|_| AppError::storage())?;
        if let Some(secret) = value.get("secret").and_then(Value::as_str) {
            guard.add(secret);
        }
        if let Some(items) = value.get("items").and_then(Value::as_array) {
            for item in items {
                if let Some(secret) = item.get("secret").and_then(Value::as_str) {
                    guard.add(secret);
                }
            }
        }
        Ok(())
    }
    /// Discovery and execution consult the same persisted permission policy.
    pub fn tool_allowed(&self, name: &str) -> Result<bool> {
        let settings = self.service.storage.settings()?;
        self.service
            .permissions
            .tool_allowed(self.caller, name, &settings, &self.service.storage)
    }
    pub fn call<T: DeserializeOwned>(&self, operation: Operation) -> Result<T> {
        serde_json::from_value(self.execute(operation)?).map_err(|_| AppError::storage())
    }
    pub fn execute(&self, operation: Operation) -> Result<Value> {
        let installing = matches!(&operation, Operation::InstallUpdate);
        // Drain MCP before taking the shared exclusive gate: a request already
        // queued on that gate must be able to finish, never deadlock the updater.
        let pause = if installing {
            self.service.permissions.authorize_operation(
                self.caller, &operation, &self.service.storage.settings()?, &self.service.storage,
            )?;
            Some(self.service.updater.pause_connections()?)
        } else { None };
        let _exclusive = if installing {
            Some(
                self.execution_gate
                    .write()
                    .map_err(|_| AppError::storage())?,
            )
        } else {
            None
        };
        let _shared = if !installing {
            Some(
                self.execution_gate
                    .read()
                    .map_err(|_| AppError::storage())?,
            )
        } else {
            None
        };
        if self.service.updater.installing() && !matches!(&operation, Operation::UpdateStatus) {
            return Err(AppError::new(
                "UPDATE_IN_PROGRESS",
                "程序正在退出以完成更新。",
            ));
        }
        let storage = &self.service.storage;
        self.service.permissions.authorize_operation(
            self.caller,
            &operation,
            &storage.settings()?,
            storage,
        )?;
        let is_mcp = self.caller == permissions::Caller::Mcp;
        if is_mcp {
            let mut guard = self.guard.write().map_err(|_| AppError::storage())?;
            self.refresh_output_guard(&mut guard)?;
            match &operation {
                Operation::Create(values) => {
                    for value in values {
                        guard.add(&value.secret);
                    }
                }
                Operation::Update(values) => {
                    for value in values {
                        if let Some(secret) = &value.secret {
                            guard.add(secret);
                        }
                    }
                }
                _ => {}
            }
        }
        let result = match operation {
            Operation::OpenUrl(url) => {
                self.service.windows.open_url(&url)?;
                Ok(Value::Null)
            }
            Operation::UpdateStatus => encoded(self.service.updater.status()?),
            Operation::CheckUpdate => encoded(self.service.updater.check()?),
            Operation::DownloadUpdate => encoded(self.service.updater.download()?),
            Operation::CancelUpdate => {
                self.service.updater.cancel();
                Ok(Value::Null)
            }
            Operation::InstallUpdate => {
                let launch = self.service.updater.prepare_install(self.directory(), pause.ok_or_else(AppError::invalid)?)?;
                if let Err(error) = self.service.windows.launch_update(launch) {
                    self.service.updater.launch_failed();
                    return Err(error);
                }
                self.service.updater.launched();
                Ok(Value::Null)
            }
            Operation::McpConfig => {
                let executable = std::env::current_exe().map_err(|_| {
                    AppError::new(
                        "CONFIG_UNAVAILABLE",
                        "Unable to locate the current executable",
                    )
                })?;
                Ok(
                    json!({"mcpServers":{"vault":{"command":configuration_path(&executable),"args":["--mcp","--data-dir",configuration_path(self.directory())]}}}),
                )
            }
            Operation::Search(args) if is_mcp => {
                ai::search(storage, &self.guard, args, &storage.settings()?)
            }
            Operation::Search(args) => {
                let entries: Vec<_> = storage
                    .list(&args.query, args.kind)?
                    .into_iter()
                    .filter(|e| {
                        args.tag.as_ref().is_none_or(|t| e.tags.contains(t))
                    })
                    .collect();
                Ok(json!({"entries":entries,"hasMore":false}))
            }
            Operation::Read(ids) => ids
                .into_iter()
                .map(|id| {
                    if is_mcp {
                        ai::metadata(storage, &self.guard, &id)
                    } else {
                        encoded(storage.summary(&id)?)
                    }
                })
                .collect::<Result<Vec<_>>>()
                .map(Value::Array),
            Operation::Create(values) => {
                self.saved(storage.create_many(values, self.caller)?, is_mcp)
            }
            Operation::Update(values) => {
                self.saved(storage.update_many(values, self.caller)?, is_mcp)
            }
            Operation::Delete(values) => encoded(
                storage
                    .delete_many(values, self.caller)?
                    .into_iter()
                    .map(|id| json!({"id":id,"status":"deleted"}))
                    .collect::<Vec<_>>(),
            ),
            Operation::Pin {
                id,
                expected_revision,
                pinned,
            } => {
                let pinned = storage.pin(&id, expected_revision, pinned)?;
                if is_mcp {
                    // MCP must never receive notes or other desktop-only fields.
                    ai::metadata(storage, &self.guard, &pinned.id)
                } else {
                    encoded(pinned)
                }
            }
            Operation::Organization => encoded(storage.organization()?),
            Operation::Organize {
                kind,
                name,
                previous,
            } => {
                if is_mcp {
                    guard_organize(storage, &name, previous.as_deref())?;
                }
                storage.organize(&kind, &name, previous.as_deref())?;
                Ok(Value::Null)
            }
            Operation::ListTrash => encoded(storage.list_trash()?),
            Operation::Restore {
                id,
                expected_revision,
            } => {
                let restored = storage.restore(&id, expected_revision)?;
                if is_mcp {
                    ai::metadata(storage, &self.guard, &restored.id)
                } else {
                    encoded(restored)
                }
            }
            Operation::EmptyTrash { owner_window } => {
                let entries = storage.list_trash()?;
                if entries.is_empty()
                    || !self
                        .service
                        .windows
                        .confirm_purge(owner_window, entries.len())?
                {
                    Ok(json!(0))
                } else {
                    encoded(
                        storage.empty_trash(
                            entries
                                .into_iter()
                                .map(|e| EntryVersion {
                                    id: e.summary.id,
                                    expected_revision: e.summary.revision,
                                })
                                .collect(),
                        )?,
                    )
                }
            }
            Operation::ReadField(args) => encoded(storage.read_field(
                &args.id,
                args.expected_revision,
                &args.field,
                args.service_id.as_deref(),
            )?),
            Operation::CopyField(args) => {
                let text = storage.read_field(
                    &args.id,
                    args.expected_revision,
                    &args.field,
                    args.service_id.as_deref(),
                )?;
                self.service.windows.copy_text(text)?;
                Ok(Value::Null)
            }
            Operation::CopyText(text) => {
                self.service.windows.copy_text(text)?;
                Ok(Value::Null)
            }
            Operation::Settings => encoded(storage.settings()?),
            Operation::PermissionPreview(settings) => encoded(
                storage
                    .list("", None)?
                    .iter()
                    .map(|e| permissions::preview(&settings, e))
                    .collect::<Vec<_>>(),
            ),
            Operation::SaveSettings(settings) => {
                storage.save_settings(&settings)?;
                encoded(settings)
            }
            Operation::StartupEnabled(executable) => encoded(
                self.service
                    .windows
                    .startup_enabled(&executable, self.directory())?,
            ),
            Operation::SetStartup {
                executable,
                enabled,
            } => encoded(self.service.windows.set_startup(
                &executable,
                self.directory(),
                enabled,
            )?),
            Operation::CaptureTarget(request) => {
                self.service.filling.capture(&self.service.windows, request)
            }
            Operation::Write(args) => {
                self.service.filling.write(storage, &self.service.windows, args)
            }
        };
        if is_mcp {
            let mut guard = self.guard.write().map_err(|_| AppError::storage())?;
            self.refresh_output_guard(&mut guard)?;
            result.map(|mut value| {
                guard.scrub(&mut value);
                value
            })
        } else {
            result
        }
    }
    fn saved(&self, entries: Vec<EntrySummary>, is_mcp: bool) -> Result<Value> {
        if is_mcp {
            encoded(
                entries
                    .into_iter()
                    .map(|e| json!({"id":e.id,"revision":e.revision,"status":"saved"}))
                    .collect::<Vec<_>>(),
            )
        } else {
            encoded(entries)
        }
    }
}

// Normalize product-owned path segments for generated configuration;
// unrelated custom directory names retain exactly the user's chosen spelling.
fn configuration_path(path: &Path) -> std::path::PathBuf {
    path.components().map(|component| {
        let value=component.as_os_str();
        if value.to_string_lossy().eq_ignore_ascii_case("vault") { std::ffi::OsString::from("vault") }
        else if value.to_string_lossy().eq_ignore_ascii_case("vault.exe") { std::ffi::OsString::from("vault.exe") }
        else { value.to_owned() }
    }).collect()
}
