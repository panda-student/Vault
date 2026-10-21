//! Storage service: business storage operations and revision checks.
use super::*;
use crate::vault::Vault;
use zeroize::Zeroizing;
pub struct StorageService {
    pub(super) vault: Vault,
}
impl StorageService {
    pub fn open(directory: &Path) -> Result<Self> {
        Ok(Self {
            vault: Vault::open(directory)?,
        })
    }
    pub fn directory(&self) -> &Path {
        self.vault.directory()
    }

    pub fn list(&self, query: &str, kind: Option<Kind>) -> Result<Vec<EntrySummary>> {
        self.vault.list(query, kind)
    }
    pub fn summary(&self, id: &str) -> Result<EntrySummary> {
        Ok(self.vault.get(id)?.summary)
    }

    pub fn create_many(
        &self,
        input: Vec<EntryInput>,
        caller: permissions::Caller,
    ) -> Result<Vec<EntrySummary>> {
        self.vault.create_many_checked(input, |s, old, new| {
            permissions::check_change(caller, s, old, new, permissions::Access::Create)
        })
    }

    pub fn update_many(
        &self,
        patches: Vec<EntryPatch>,
        caller: permissions::Caller,
    ) -> Result<Vec<EntrySummary>> {
        self.vault.update_many_checked(patches, |s, old, new| {
            permissions::check_change(caller, s, old, new, permissions::Access::Update)
        })
    }

    pub fn delete_many(
        &self,
        entries: Vec<EntryVersion>,
        caller: permissions::Caller,
    ) -> Result<Vec<String>> {
        self.vault.delete_many_checked(entries, |s, old, new| {
            permissions::check_change(caller, s, old, new, permissions::Access::Delete)
        })
    }
    pub fn pin(&self, id: &str, revision: i64, pinned: bool) -> Result<EntrySummary> {
        self.vault.pin(id, revision, pinned)
    }
    pub fn organization(&self) -> Result<Organization> {
        self.vault.organization()
    }
    pub fn organize(&self, kind: &str, name: &str, previous: Option<&str>) -> Result<()> {
        self.vault.organize(kind, name, previous)
    }
    pub fn list_trash(&self) -> Result<Vec<TrashEntry>> {
        self.vault.list_trash()
    }
    pub fn restore(&self, id: &str, revision: i64) -> Result<EntrySummary> {
        self.vault.restore(id, revision)
    }
    /// Core interface supplies the exact snapshot after Windows-service confirmation.
    pub fn empty_trash(&self, expected: Vec<EntryVersion>) -> Result<usize> {
        self.vault.empty_trash(expected)
    }
    pub fn settings(&self) -> Result<Settings> {
        self.vault.settings()
    }
    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.vault.save_settings(settings)
    }
    pub fn protected_values(&self) -> Result<Zeroizing<Vec<String>>> {
        self.vault.protected_values()
    }
    /// Human desktop operations only. MCP exposes summary metadata and local writes.
    pub fn read_field(
        &self,
        id: &str,
        revision: i64,
        field: &str,
        service: Option<&str>,
    ) -> Result<String> {
        let entry = self.vault.get(id)?;
        if entry.summary.revision != revision {
            return Err(AppError::conflict());
        }
        Ok(match field {
            "account" => entry.summary.account,
            "url" if service.is_some() => entry.summary.service(service)?.url.clone(),
            "url" => entry.summary.url,
            "secret" => entry.secret,
            _ => return Err(AppError::invalid()),
        })
    }
}
