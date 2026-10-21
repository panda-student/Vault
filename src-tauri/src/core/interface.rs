//! Transport-neutral operation contract. Caller identity is never deserialized.
use super::*;
use serde::Deserialize;
use std::path::PathBuf;

/// Desktop supplies the OS clipboard adapter; validation and permission remain core-owned.
pub trait ClipboardPort: Send + Sync {
    fn write_text(&self, text: String) -> Result<()>;
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Search {
    #[serde(default)]
    pub query: String,
    pub kind: Option<Kind>,
    pub group: Option<String>,
    pub tag: Option<String>,
}
impl Search {
    pub fn new(query: String, kind: Option<Kind>) -> Self {
        Self {
            query,
            kind,
            group: None,
            tag: None,
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FieldRequest {
    pub id: String,
    pub expected_revision: i64,
    pub field: String,
    pub service_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FillRequest {
    pub credential_id: String,
    pub field: String,
    pub service_id: Option<String>,
    pub target_id: String,
}

pub enum Operation {
    UpdateStatus,
    CheckUpdate,
    DownloadUpdate,
    CancelUpdate,
    InstallUpdate,
    McpConfig,
    Search(Search),
    Read(Vec<String>),
    Create(Vec<EntryInput>),
    Update(Vec<EntryPatch>),
    Delete(Vec<EntryVersion>),
    Pin {
        id: String,
        expected_revision: i64,
        pinned: bool,
    },
    Organization,
    Organize {
        kind: String,
        name: String,
        previous: Option<String>,
    },
    ListTrash,
    Restore {
        id: String,
        expected_revision: i64,
    },
    /// Window handle is supplied by the trusted desktop adapter, never a request boolean.
    EmptyTrash {
        owner_window: isize,
    },
    ReadField(FieldRequest),
    CopyField(FieldRequest),
    CopyText(String),
    Settings,
    SaveSettings(Settings),
    PermissionPreview(Settings),
    StartupEnabled(PathBuf),
    SetStartup {
        executable: PathBuf,
        enabled: bool,
    },
    CaptureTarget,
    Write(WriteArgs),
    Fill(FillRequest),
}
