//! Default -> tag -> entry policy, owned by the core.
use super::*;
use crate::vault::PermissionRule;
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Caller {
    Desktop,
    Mcp,
}
pub(super) struct PermissionService;
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Access {
    Public,
    Read,
    Create,
    Update,
    Delete,
    Input,
    File,
    DesktopOnly,
}
impl Access {
    fn rule(self, r: &PermissionRule) -> Option<bool> {
        match self {
            Self::Read => r.read_metadata,
            Self::Create => r.create,
            Self::Update => r.update,
            Self::Delete => r.delete,
            Self::Input => r.fill_input,
            Self::File => r.write_file,
            _ => None,
        }
    }
    fn default(self, s: &Settings) -> bool {
        let p = &s.mcp_permissions;
        match self {
            Self::Public => true,
            Self::Read => p.read_metadata,
            Self::Create => p.create,
            Self::Update => p.update,
            Self::Delete => p.delete,
            Self::Input => p.fill_input,
            Self::File => p.write_file,
            Self::DesktopOnly => false,
        }
    }
}
fn denied() -> AppError {
    AppError::new(
        "PERMISSION_DENIED",
        "此操作未获授权，请在桌面 MCP 权限管理中查看生效规则。",
    )
}
pub(super) fn preview(s: &Settings, e: &EntrySummary) -> Value {
    let entry = s.mcp_entry_rules.get(&e.id);
    let mut values = serde_json::Map::new();
    for (key, a) in [
        ("access", Access::Public),
        ("readMetadata", Access::Read),
        ("create", Access::Create),
        ("update", Access::Update),
        ("delete", Access::Delete),
        ("fillInput", Access::Input),
        ("writeFile", Access::File),
    ] {
        let pick = |r: &PermissionRule| if key == "access" { r.access } else { a.rule(r) };
        let explicit = entry.and_then(pick);
        let tag_values: Vec<_> = e
            .tags
            .iter()
            .filter_map(|tag| s.mcp_tag_rules.get(tag).and_then(pick).map(|v| (tag, v)))
            .collect();
        let denied_tags: Vec<_> = tag_values
            .iter()
            .filter(|(_, v)| !*v)
            .map(|(t, _)| t.as_str())
            .collect();
        let source = if explicit.is_some() {
            "单条设置".to_string()
        } else if !denied_tags.is_empty() {
            format!(
                "标签：{}{}",
                denied_tags.join("、"),
                if tag_values.iter().any(|(_, v)| *v) {
                    "（禁止优先）"
                } else {
                    ""
                }
            )
        } else if !tag_values.is_empty() {
            format!(
                "标签：{}",
                tag_values
                    .iter()
                    .map(|(t, _)| t.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            )
        } else {
            "默认规则".into()
        };
        let configured = resolve(
            s,
            Some(&e.id),
            &e.tags,
            pick,
            if key == "access" {
                s.mcp_access
            } else {
                a.default(s)
            },
        );
        values.insert(key.into(),json!({"configured":configured,"allowed":permitted(s,Some(&e.id),&e.tags,a),"source":if !s.mcp_enabled{"MCP 已关闭".into()}else{source}}));
    }
    json!({"id":e.id,"permissions":values})
}
fn enabled(s: &Settings) -> Result<()> {
    if s.mcp_enabled {
        Ok(())
    } else {
        Err(AppError::new("MCP_DISABLED", "本机界面已关闭 MCP 访问。"))
    }
}
fn resolve(
    s: &Settings,
    id: Option<&str>,
    tags: &[String],
    pick: impl Fn(&PermissionRule) -> Option<bool>,
    fallback: bool,
) -> bool {
    if let Some(value) = id.and_then(|id| s.mcp_entry_rules.get(id)).and_then(&pick) {
        return value;
    }
    let values: Vec<_> = tags
        .iter()
        .filter_map(|t| s.mcp_tag_rules.get(t))
        .filter_map(pick)
        .collect();
    if values.contains(&false) {
        false
    } else if !values.is_empty() {
        true
    } else {
        fallback
    }
}
pub(super) fn permitted(s: &Settings, id: Option<&str>, tags: &[String], a: Access) -> bool {
    s.mcp_enabled
        && a != Access::DesktopOnly
        && resolve(s, id, tags, |r| r.access, s.mcp_access)
        && resolve(s, id, tags, |r| a.rule(r), a.default(s))
}
pub(super) fn check_change(
    caller: Caller,
    s: &Settings,
    old: Option<&EntrySummary>,
    new: &EntrySummary,
    a: Access,
) -> Result<()> {
    if caller == Caller::Desktop {
        return Ok(());
    }
    enabled(s)?;
    if !permitted(s, Some(&new.id), &new.tags, a)
        || old.is_some_and(|e| !permitted(s, Some(&e.id), &e.tags, a))
    {
        return Err(denied());
    }
    // MCP cannot relabel an existing item to grant itself additional capabilities.
    if let Some(old) = old
        && old.tags != new.tags
    {
        for action in [
            Access::Read,
            Access::Create,
            Access::Update,
            Access::Delete,
            Access::Input,
            Access::File,
        ] {
            if permitted(s, Some(&new.id), &new.tags, action)
                && !permitted(s, Some(&old.id), &old.tags, action)
            {
                return Err(denied());
            }
        }
    }
    Ok(())
}
impl PermissionService {
    fn available(
        &self,
        a: Access,
        s: &Settings,
        storage: &storage::StorageService,
    ) -> Result<bool> {
        if !s.mcp_enabled {
            return Ok(false);
        }
        if a == Access::Public {
            return Ok(true);
        }
        if a == Access::DesktopOnly {
            return Ok(false);
        }
        if permitted(s, None, &[], a) {
            return Ok(true);
        }
        if a == Access::Create {
            return Ok(s
                .mcp_tag_rules
                .keys()
                .any(|t| permitted(s, None, std::slice::from_ref(t), a)));
        }
        Ok(storage
            .list("", None)?
            .iter()
            .any(|e| permitted(s, Some(&e.id), &e.tags, a)))
    }
    pub fn authorize_operation(
        &self,
        caller: Caller,
        op: &Operation,
        s: &Settings,
        storage: &storage::StorageService,
    ) -> Result<()> {
        if caller == Caller::Desktop {
            return Ok(());
        }
        enabled(s)?;
        let a = access(op);
        if a == Access::DesktopOnly {
            return Err(denied());
        }
        match op {
            Operation::Create(_) | Operation::Update(_) | Operation::Delete(_) => Ok(()),
            Operation::Read(ids) => {
                for id in ids {
                    let e = storage.summary(id)?;
                    if !permitted(s, Some(id), &e.tags, a) {
                        return Err(denied());
                    }
                }
                Ok(())
            }
            Operation::Write(args) => {
                let e = storage.summary(args.credential_id())?;
                if permitted(s, Some(&e.id), &e.tags, a) {
                    Ok(())
                } else {
                    Err(denied())
                }
            }
            Operation::Fill(args) => {
                let e = storage.summary(&args.credential_id)?;
                if permitted(s, Some(&e.id), &e.tags, a) {
                    Ok(())
                } else {
                    Err(denied())
                }
            }
            Operation::Pin { id, .. } => {
                let e = storage.summary(id)?;
                if permitted(s, Some(&e.id), &e.tags, a) {
                    Ok(())
                } else {
                    Err(denied())
                }
            }
            Operation::Restore { id, .. } => {
                let e = storage
                    .list_trash()?
                    .into_iter()
                    .map(|t| t.summary)
                    .find(|e| e.id == *id)
                    .ok_or(AppError::new("NOT_FOUND", "垃圾桶条目不存在。"))?;
                if permitted(s, Some(&e.id), &e.tags, a) {
                    Ok(())
                } else {
                    Err(denied())
                }
            }
            // Global tag management mutates every entry carrying the tag, so it
            // requires the default update grant, not just one entry's rule.
            Operation::Organize { name, previous, .. } => {
                if !permitted(s, None, &[], a) {
                    return Err(denied());
                }
                if let Some(old) = previous {
                    let affected = storage
                        .list("", None)?
                        .into_iter()
                        .chain(storage.list_trash()?.into_iter().map(|t| t.summary))
                        .filter(|e| e.tags.iter().any(|t| t == old))
                        .collect::<Vec<_>>();
                    // Empty tags can intentionally exist in the persisted catalog.
                    // Entry references are used for authorization, not existence.
                    if name.trim().is_empty()
                        && !storage.organization()?.tags.iter().any(|tag| tag == old)
                    {
                        return Err(AppError::new("NOT_FOUND", "标签不存在。"));
                    }
                    for entry in affected {
                        if !permitted(s, Some(&entry.id), &entry.tags, a) {
                            return Err(denied());
                        }
                    }
                }
                Ok(())
            }
            _ => {
                if self.available(a, s, storage)? {
                    Ok(())
                } else {
                    Err(denied())
                }
            }
        }
    }
    pub fn tool_allowed(
        &self,
        caller: Caller,
        name: &str,
        s: &Settings,
        storage: &storage::StorageService,
    ) -> Result<bool> {
        let Some(a) = tool_access(name, s) else {
            return Ok(false);
        };
        if caller == Caller::Desktop {
            return Ok(true);
        }
        if name == "write_credential" {
            return Ok(self.available(Access::File, s, storage)?
                || self.available(Access::Input, s, storage)?);
        }
        self.available(a, s, storage)
    }
}
pub(super) fn access(op: &Operation) -> Access {
    match op {
        Operation::McpConfig => Access::Public,
        Operation::Search(_) | Operation::Read(_) => Access::Read,
        Operation::Create(_) => Access::Create,
        Operation::Update(_)
        | Operation::Pin { .. }
        | Operation::Restore { .. }
        | Operation::Organize { .. } => Access::Update,
        Operation::Delete(_) => Access::Delete,
        Operation::Write(args) => {
            if args.is_file() {
                Access::File
            } else {
                Access::Input
            }
        }
        Operation::Fill(_) | Operation::CaptureTarget(_) => Access::Input,
        _ => Access::DesktopOnly,
    }
}
pub(super) fn tool_access(name: &str, _s: &Settings) -> Option<Access> {
    Some(match name {
        "get_mcp_config" => Access::Public,
        "search_entries" | "get_entries" => Access::Read,
        "create_entries" => Access::Create,
        "update_entries" | "restore_entries" | "pin_entries" | "organize" => Access::Update,
        "delete_entries" => Access::Delete,
        "capture_fill_target" | "fill_credential" | "write_credential" => Access::Input,
        _ => return None,
    })
}
