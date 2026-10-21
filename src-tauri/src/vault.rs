#[cfg(test)]
mod migration_tests;
mod organization;
mod policy;
pub use organization::Organization;
mod trash;
use crate::{
    crypto,
    error::{AppError, Result},
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
pub use trash::TrashEntry;
use zeroize::Zeroizing;

pub use models::*;
mod models;

pub struct Vault {
    directory: PathBuf,
    key: Zeroizing<Vec<u8>>,
}
struct Row {
    id: String,
    packed: Vec<u8>,
    revision: i64,
    pinned: bool,
    created_at: i64,
    updated_at: i64,
}

impl Row {
    fn aad(&self) -> Vec<u8> {
        format!(
            "local-password-v1:{}:{}:{}:{}:{}",
            self.id, self.revision, self.pinned, self.created_at, self.updated_at
        )
        .into_bytes()
    }
    fn summary(&self, value: &EntryInput) -> EntrySummary {
        EntrySummary {
            id: self.id.clone(),
            kind: value.kind,
            title: value.title.clone(),
            group: value.group.clone(),
            tags: value.tags.clone(),
            database: value.database.clone(),
            account: value.account.clone(),
            url: value.url.clone(),
            services: value.services.clone(),
            notes: value.notes.clone(),
            pinned: self.pinned,
            created_at: self.created_at,
            updated_at: self.updated_at,
            revision: self.revision,
        }
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
fn from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: row.get(0)?,
        packed: row.get(1)?,
        revision: row.get(2)?,
        pinned: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}
const COLUMNS: &str = "id, encrypted, revision, pinned, created_at, updated_at";
const KEY_AAD: &[u8] = b"local-password-manager-key-check-v1";

impl Vault {
    pub fn open(directory: &Path) -> Result<Self> {
        if !directory.is_absolute() {
            return Err(AppError::invalid());
        }
        std::fs::create_dir_all(directory).map_err(|_| AppError::storage())?;
        let directory = directory.canonicalize().map_err(|_| AppError::storage())?;
        let path = directory.join("vault.db");
        let existed = path.exists();
        if existed
            && std::fs::metadata(&path)
                .map_err(|_| AppError::storage())?
                .len()
                == 0
        {
            return Err(AppError::new(
                "INVALID_DATABASE",
                "已有密码库为空或不完整，请保留原文件并检查备份。",
            ));
        }
        let mut connection = Connection::open(&path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        // Only encrypted content reaches SQLite, including its WAL and free pages.
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA secure_delete=ON;",
        )?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = transaction.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let key = if version == 0 && !existed {
            let tables: i64 = transaction.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |row| row.get(0),
            )?;
            if tables != 0 {
                return Err(AppError::new(
                    "INVALID_DATABASE",
                    "该文件不是本应用的密码库，未进行初始化。",
                ));
            }
            transaction.execute_batch("CREATE TABLE metadata (name TEXT PRIMARY KEY, value BLOB NOT NULL); CREATE TABLE entries (id TEXT PRIMARY KEY, encrypted BLOB NOT NULL, revision INTEGER NOT NULL, pinned INTEGER NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL); PRAGMA user_version=1;")?;
            let key = Zeroizing::new(crypto::random::<32>()?.to_vec());
            let wrapped = crypto::protect(&key)?;
            transaction.execute(
                "INSERT INTO metadata (name,value) VALUES ('wrapped_key',?1)",
                [wrapped],
            )?;
            let check = crypto::encrypt(&key, KEY_AAD, KEY_AAD)?;
            transaction.execute(
                "INSERT INTO metadata (name,value) VALUES ('key_check',?1)",
                [check],
            )?;
            let settings =
                serde_json::to_vec(&Settings::default()).map_err(|_| AppError::storage())?;
            transaction.execute(
                "INSERT INTO metadata (name,value) VALUES ('settings',?1)",
                [settings],
            )?;
            key
        } else {
            if version != 1 && version != 2 && version != 3 {
                return Err(AppError::new(
                    "UNSUPPORTED_DATABASE",
                    "密码库格式不受支持，原文件未被覆盖。",
                ));
            }
            let wrapped: Vec<u8> = transaction.query_row(
                "SELECT value FROM metadata WHERE name='wrapped_key'",
                [],
                |r| r.get(0),
            )?;
            let key = crypto::unprotect(&wrapped)?;
            if key.len() != 32 {
                return Err(AppError::corrupt());
            }
            let packed: Vec<u8> = transaction.query_row(
                "SELECT value FROM metadata WHERE name='key_check'",
                [],
                |r| r.get(0),
            )?;
            if crypto::decrypt(&key, KEY_AAD, &packed)?.as_slice() != KEY_AAD {
                return Err(AppError::corrupt());
            }
            key
        };
        transaction.execute_batch("CREATE TABLE IF NOT EXISTS trash (id TEXT PRIMARY KEY, encrypted BLOB NOT NULL, revision INTEGER NOT NULL, pinned INTEGER NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER NOT NULL);")?;
        if version < 2 {
            transaction.execute_batch("PRAGMA user_version=2;")?;
        }
        transaction.commit()?;
        if existed && version < 3 {
            let backups = directory.join("backups");
            std::fs::create_dir_all(&backups).map_err(|_| AppError::storage())?;
            let backup = backups.join(format!(
                "before-tags-permissions-{}.db",
                uuid::Uuid::new_v4()
            ));
            connection.execute("VACUUM INTO ?1", [backup.to_string_lossy().as_ref()])?;
        }
        let vault = Self { directory, key };
        vault.migrate_labels()?;
        Ok(vault)
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn protected_values(&self) -> Result<Zeroizing<Vec<String>>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS} FROM entries UNION ALL SELECT {COLUMNS} FROM trash"
        ))?;
        let mut values = Zeroizing::new(Vec::new());
        for row in statement.query_map([], from_sql)? {
            values.push(self.decode(&row?)?.secret);
        }
        let mut history = connection
            .prepare("SELECT name,value FROM metadata WHERE name GLOB 'retired_secret:*'")?;
        for row in history.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })? {
            let (name, packed) = row?;
            let value = crypto::decrypt(&self.key, name.as_bytes(), &packed)?;
            values.push(String::from_utf8(value.to_vec()).map_err(|_| AppError::corrupt())?);
        }
        Ok(values)
    }
    fn connection(&self) -> Result<Connection> {
        let connection = Connection::open_with_flags(
            self.directory.join("vault.db"),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_secs(5))?;
        Ok(connection)
    }
    fn retain_protected_value(&self, connection: &Connection, id: &str, value: &str) -> Result<()> {
        let name = format!("retired_secret:{}:{}", id, uuid::Uuid::new_v4());
        let packed = crypto::encrypt(&self.key, name.as_bytes(), value.as_bytes())?;
        connection.execute(
            "INSERT INTO metadata(name,value) VALUES (?1,?2)",
            params![name, packed],
        )?;
        Ok(())
    }
    fn decode(&self, row: &Row) -> Result<EntryInput> {
        let plaintext = crypto::decrypt(&self.key, &row.aad(), &row.packed)?;
        let mut value: EntryInput =
            serde_json::from_slice(&plaintext).map_err(|_| AppError::corrupt())?;
        if value.kind == Kind::Api && value.services.is_empty() && !value.url.is_empty() {
            value.services.push(ApiService {
                id: "legacy".into(),
                protocol: ApiProtocol::Custom,
                url: value.url.clone(),
            });
        }
        for service in &mut value.services {
            if matches!(service.protocol, ApiProtocol::Figma | ApiProtocol::Gitea) {
                service.protocol = ApiProtocol::Custom;
            }
        }
        Self::normalize_labels(&mut value);
        Ok(value)
    }
    fn encode(&self, row: &Row, value: &EntryInput) -> Result<Vec<u8>> {
        let plaintext = Zeroizing::new(serde_json::to_vec(value).map_err(|_| AppError::invalid())?);
        crypto::encrypt(&self.key, &row.aad(), &plaintext)
    }
    fn normalize_services(value: &mut EntryInput) -> Result<()> {
        Self::normalize_labels(value);
        value.group = value.group.trim().into();
        value.tags = value.tags.iter().map(|t| t.trim().to_string()).collect();
        let mut tags = std::collections::HashSet::new();
        value.tags.retain(|t| tags.insert(t.clone()));
        if value.kind == Kind::Api {
            if value.services.is_empty() && !value.url.is_empty() {
                value.services.push(ApiService {
                    id: "legacy".into(),
                    protocol: ApiProtocol::Custom,
                    url: value.url.clone(),
                });
            }
            for service in &mut value.services {
                service.url = service.url.trim().into();
                if service.id.is_empty() {
                    service.id = uuid::Uuid::new_v4().to_string();
                }
            }
            value.url = value
                .services
                .first()
                .map(|s| s.url.clone())
                .unwrap_or_default();
        }
        let mut ids = std::collections::HashSet::new();
        if value.services.len() > 20
            || value.kind != Kind::Api && !value.services.is_empty()
            || value.services.iter().any(|s| {
                s.url.is_empty() || s.url.len() > 8192 || s.id.len() > 64 || !ids.insert(&s.id)
            })
        {
            return Err(AppError::invalid());
        }
        Ok(())
    }
    fn validate(value: &EntryInput) -> Result<()> {
        if value.group.len() > 256
            || value.tags.len() > 64
            || value
                .tags
                .iter()
                .any(|t| t.trim().is_empty() || t.len() > 256)
            || (value.kind != Kind::Database && value.database.is_some())
        {
            return Err(AppError::invalid());
        }
        if value.kind == Kind::Database {
            let db = value.database.as_ref().ok_or_else(AppError::invalid)?;
            if db.database_type.trim().is_empty()
                || db.database_type.len() > 128
                || db.host.len() > 4096
                || db.database_name.len() > 4096
                || (!db.port.is_empty() && db.port.parse::<u16>().ok().filter(|p| *p > 0).is_none())
            {
                return Err(AppError::invalid());
            }
        }
        if value.title.trim().is_empty()
            || (value.secret.is_empty() && matches!(value.kind, Kind::Password | Kind::Api))
            || value.title.len() > 1024
            || value.account.len() > 4096
            || value.url.len() > 8192
            || value.secret.len() > 131072
            || value.notes.len() > 65536
        {
            return Err(AppError::invalid());
        }
        Ok(())
    }
    fn find(connection: &Connection, id: &str) -> Result<Row> {
        if id.len() > 64 {
            return Err(AppError::invalid());
        }
        connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM entries WHERE id=?1"),
                [id],
                from_sql,
            )
            .optional()?
            .ok_or(AppError::new("NOT_FOUND", "条目不存在，可能已被删除。"))
    }
    pub fn list(&self, query: &str, kind: Option<Kind>) -> Result<Vec<EntrySummary>> {
        if query.len() > 4096 {
            return Err(AppError::invalid());
        }
        let connection = self.connection()?;
        let mut statement = connection.prepare(&format!("SELECT {COLUMNS} FROM entries"))?;
        let rows = statement.query_map([], from_sql)?;
        let query = query.trim().to_lowercase();
        let mut result = Vec::new();
        for row in rows {
            let row = row?;
            let value = self.decode(&row)?;
            if kind.is_none_or(|k| k == value.kind)
                && ([&value.title, &value.account, &value.url, &value.group]
                    .iter()
                    .any(|s| s.to_lowercase().contains(&query))
                    || value.tags.iter().any(|s| s.to_lowercase().contains(&query))
                    || value.database.as_ref().is_some_and(|db| {
                        [&db.host, &db.database_type, &db.database_name]
                            .iter()
                            .any(|s| s.to_lowercase().contains(&query))
                    })
                    || value
                        .services
                        .iter()
                        .any(|s| s.url.to_lowercase().contains(&query)))
            {
                result.push(row.summary(&value));
            }
        }
        result.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(result)
    }
    pub fn get(&self, id: &str) -> Result<EntryDetail> {
        let row = Self::find(&self.connection()?, id)?;
        let value = self.decode(&row)?;
        Ok(EntryDetail {
            summary: row.summary(&value),
            secret: value.secret,
        })
    }
    fn write_many<T, R>(
        &self,
        values: Vec<T>,
        mut action: impl FnMut(&Connection, T) -> Result<R>,
    ) -> Result<Vec<R>> {
        if values.is_empty() || values.len() > 100 {
            return Err(AppError::invalid());
        }
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut result = Vec::with_capacity(values.len());
        for value in values {
            result.push(action(&tx, value)?);
        }
        tx.commit()?;
        Ok(result)
    }
    #[cfg(test)]
    pub fn create(&self, value: EntryInput) -> Result<EntrySummary> {
        Ok(self.create_many(vec![value])?.remove(0))
    }
    #[cfg(test)]
    pub fn create_many(&self, values: Vec<EntryInput>) -> Result<Vec<EntrySummary>> {
        self.create_many_checked(values, |_, _, _| Ok(()))
    }
    pub fn create_many_checked(
        &self,
        values: Vec<EntryInput>,
        check: impl Fn(&Settings, Option<&EntrySummary>, &EntrySummary) -> Result<()>,
    ) -> Result<Vec<EntrySummary>> {
        self.write_many(values, |connection, mut value| {
        value.title = value.title.trim().into();
        if value.kind == Kind::Api && !value.url.is_empty() && value.services.first().is_some_and(|first| first.url.trim()!=value.url.trim()) { return Err(AppError::invalid()); }
        Self::normalize_services(&mut value)?;
        Self::validate(&value)?;
        let time = now();
        let row = Row {
            id: uuid::Uuid::new_v4().to_string(),
            packed: vec![],
            revision: 1,
            pinned: false,
            created_at: time,
            updated_at: time,
        };
        let packed = self.encode(&row, &value)?;
        check(&self.settings_on(connection)?,None,&row.summary(&value))?;
        connection.execute("INSERT INTO entries (id,encrypted,revision,pinned,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6)",params![row.id,packed,row.revision,row.pinned,row.created_at,row.updated_at])?;
        Ok(row.summary(&value))
        })
    }
    #[cfg(test)]
    pub fn update(&self, patch: EntryPatch) -> Result<EntrySummary> {
        Ok(self.update_many(vec![patch])?.remove(0))
    }
    #[cfg(test)]
    pub fn update_many(&self, values: Vec<EntryPatch>) -> Result<Vec<EntrySummary>> {
        self.update_many_checked(values, |_, _, _| Ok(()))
    }
    pub fn update_many_checked(
        &self,
        values: Vec<EntryPatch>,
        check: impl Fn(&Settings, Option<&EntrySummary>, &EntrySummary) -> Result<()>,
    ) -> Result<Vec<EntrySummary>> {
        let mut ids = std::collections::HashSet::new();
        if values.iter().any(|v| !ids.insert(&v.id)) {
            return Err(AppError::invalid());
        }
        self.write_many(values, |connection, patch| {
            let mut row = Self::find(connection, &patch.id)?;
            if row.revision != patch.expected_revision {
                return Err(AppError::conflict());
            }
            let mut value = self.decode(&row)?;
            let original = row.summary(&value);
            let old_secret = Zeroizing::new(value.secret.clone());
            if let Some(v) = patch.title {
                value.title = v.trim().into();
            }
            if let Some(v) = patch.group {
                value.group = v.trim().into();
            }
            if let Some(v) = patch.tags {
                value.tags = v;
            }
            if let Some(v) = patch.database {
                value.database = Some(v);
            }
            if let Some(v) = patch.account {
                value.account = v;
            }
            if let Some(services) = patch.services {
                if patch.url.as_ref().is_some_and(|url| {
                    services.first().map(|s| s.url.trim()).unwrap_or("") != url.trim()
                }) {
                    return Err(AppError::invalid());
                }
                value.services = services;
                value.url.clear();
            } else if let Some(v) = patch.url {
                if value.kind == Kind::Api {
                    if v.trim().is_empty() && !value.services.is_empty() {
                        value.services.remove(0);
                    } else if let Some(first) = value.services.first_mut() {
                        first.url = v.clone();
                    }
                }
                value.url = v;
            }
            Self::normalize_services(&mut value)?;
            if let Some(v) = patch.secret {
                value.secret = v;
            }
            if let Some(v) = patch.notes {
                value.notes = v;
            }
            Self::validate(&value)?;
            check(
                &self.settings_on(connection)?,
                Some(&original),
                &row.summary(&value),
            )?;
            if old_secret.as_str() != value.secret {
                self.retain_protected_value(connection, &row.id, &old_secret)?;
            }
            row.revision += 1;
            row.updated_at = now();
            let packed = self.encode(&row, &value)?;
            connection.execute(
                "UPDATE entries SET encrypted=?1,revision=?2,updated_at=?3 WHERE id=?4",
                params![packed, row.revision, row.updated_at, row.id],
            )?;
            Ok(row.summary(&value))
        })
    }
    pub fn pin(&self, id: &str, expected_revision: i64, pinned: bool) -> Result<EntrySummary> {
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut row = Self::find(&tx, id)?;
        if row.revision != expected_revision {
            return Err(AppError::conflict());
        }
        let value = self.decode(&row)?;
        row.pinned = pinned;
        row.revision += 1;
        row.updated_at = now();
        let packed = self.encode(&row, &value)?;
        tx.execute(
            "UPDATE entries SET encrypted=?1,revision=?2,pinned=?3,updated_at=?4 WHERE id=?5",
            params![packed, row.revision, row.pinned, row.updated_at, row.id],
        )?;
        tx.commit()?;
        Ok(row.summary(&value))
    }
    #[cfg(test)]
    pub fn delete(&self, id: &str, expected_revision: i64) -> Result<()> {
        self.delete_many(vec![EntryVersion {
            id: id.to_owned(),
            expected_revision,
        }])
        .map(|_| ())
    }
    #[cfg(test)]
    pub fn delete_many(&self, values: Vec<EntryVersion>) -> Result<Vec<String>> {
        self.delete_many_checked(values, |_, _, _| Ok(()))
    }
    pub fn delete_many_checked(
        &self,
        values: Vec<EntryVersion>,
        check: impl Fn(&Settings, Option<&EntrySummary>, &EntrySummary) -> Result<()>,
    ) -> Result<Vec<String>> {
        let mut ids = std::collections::HashSet::new();
        if values.iter().any(|v| !ids.insert(&v.id)) {
            return Err(AppError::invalid());
        }
        self.write_many(values, |connection, item| {
            let row = Self::find(connection, &item.id)?;
            let original = row.summary(&self.decode(&row)?);
            check(&self.settings_on(connection)?,Some(&original),&original)?;
            if row.revision != item.expected_revision {
                return Err(AppError::conflict());
            }
            // Move the authenticated encrypted record, never decrypt into SQLite.
            connection.execute(
                "INSERT INTO trash (id,encrypted,revision,pinned,created_at,updated_at,deleted_at) SELECT id,encrypted,revision,pinned,created_at,updated_at,?2 FROM entries WHERE id=?1",
                params![item.id, now()],
            )?;
            connection.execute("DELETE FROM entries WHERE id=?1", [&item.id])?;
            connection.execute(
                "DELETE FROM metadata WHERE name=?1",
                [format!("api_origin:{}", item.id)],
            )?;
            Ok(item.id)
        })
    }
    pub fn settings(&self) -> Result<Settings> {
        self.settings_on(&self.connection()?)
    }
    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        self.save_settings_on(&tx, settings)?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_and_multiple_service_addresses_roundtrip_without_losing_secret() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let entry = vault.create(value()).unwrap();
        let connection = vault.connection().unwrap();
        let row = Vault::find(&connection, &entry.id).unwrap();
        let mut old_json = serde_json::to_value(value()).unwrap();
        old_json.as_object_mut().unwrap().remove("services");
        let packed = crypto::encrypt(
            &vault.key,
            &row.aad(),
            &serde_json::to_vec(&old_json).unwrap(),
        )
        .unwrap();
        connection
            .execute(
                "UPDATE entries SET encrypted=?1 WHERE id=?2",
                params![packed, entry.id],
            )
            .unwrap();
        assert_eq!(
            vault.get(&entry.id).unwrap().summary.services[0].id,
            "legacy"
        );
        let patch: EntryPatch = serde_json::from_value(serde_json::json!({"id":entry.id,"expectedRevision":1,"services":[{"protocol":"openai","url":"https://first-protocol-fixture.invalid/v1"},{"protocol":"gemini","url":"https://second-protocol-fixture.invalid/v1beta"}]})).unwrap();
        let updated = vault.update(patch).unwrap();
        assert_eq!(updated.services.len(), 2);
        assert_ne!(updated.services[0].id, updated.services[1].id);
        assert_eq!(updated.url, updated.services[0].url);
        assert_eq!(updated.service(None).unwrap_err().code, "SERVICE_REQUIRED");
        let second_id = updated.services[1].id.clone();
        assert_eq!(
            updated.service(Some(&second_id)).unwrap().protocol,
            ApiProtocol::Gemini
        );
        let patch:EntryPatch=serde_json::from_value(serde_json::json!({"id":entry.id,"expectedRevision":2,"url":"https://updated-first-fixture.invalid/v1"})).unwrap();
        vault.update(patch).unwrap();
        drop(connection);
        drop(vault);
        let vault = Vault::open(dir.path()).unwrap();
        let current = vault.get(&entry.id).unwrap();
        assert_eq!(current.secret, value().secret);
        assert_eq!(current.summary.services[1].id, second_id);
        assert_eq!(
            current.summary.services[0].url,
            "https://updated-first-fixture.invalid/v1"
        );
        let bytes = std::fs::read(dir.path().join("vault.db")).unwrap();
        assert!(
            !bytes
                .windows(b"second-protocol-fixture".len())
                .any(|w| w == b"second-protocol-fixture")
        );
        let patch: EntryPatch = serde_json::from_value(
            serde_json::json!({"id":entry.id,"expectedRevision":3,"services":[]}),
        )
        .unwrap();
        let cleared = vault.update(patch).unwrap();
        assert!(cleared.services.is_empty() && cleared.url.is_empty());
    }
    #[test]
    fn invalid_service_sets_do_not_commit_partial_batch() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let mut duplicate = value();
        duplicate.url.clear();
        duplicate.services = vec![
            ApiService {
                id: "same".into(),
                protocol: ApiProtocol::Openai,
                url: "https://a.invalid/v1".into(),
            },
            ApiService {
                id: "same".into(),
                protocol: ApiProtocol::Gemini,
                url: "https://b.invalid/v1beta".into(),
            },
        ];
        assert!(vault.create_many(vec![value(), duplicate]).is_err());
        assert!(vault.list("", None).unwrap().is_empty());
        let mut wrong_kind = value();
        wrong_kind.kind = Kind::Password;
        wrong_kind.services = vec![ApiService {
            id: "one".into(),
            protocol: ApiProtocol::Custom,
            url: "https://a.invalid".into(),
        }];
        assert!(vault.create(wrong_kind).is_err());
    }
    #[test]
    fn batch_writes_roll_back_and_remain_compatible_with_single_operations() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let mut bad = value();
        bad.secret.clear();
        assert!(vault.create_many(vec![value(), bad]).is_err());
        assert!(vault.list("", None).unwrap().is_empty());
        let entries = vault.create_many(vec![value(), value()]).unwrap();
        let patch = |id: &str, revision: i64| {
            serde_json::from_value::<EntryPatch>(serde_json::json!({"id":id,"expectedRevision":revision,"title":"batch-updated","secret":"batch-rotated-value"})).unwrap()
        };
        assert_eq!(
            vault
                .update_many(vec![patch(&entries[0].id, 1), patch(&entries[1].id, 99)])
                .unwrap_err()
                .code,
            "CONFLICT"
        );
        assert_eq!(vault.get(&entries[0].id).unwrap().summary.revision, 1);
        assert_eq!(vault.get(&entries[0].id).unwrap().secret, value().secret);
        assert!(
            !vault
                .protected_values()
                .unwrap()
                .iter()
                .any(|v| v == "batch-rotated-value")
        );
        let versions = |revision| {
            vec![
                EntryVersion {
                    id: entries[0].id.clone(),
                    expected_revision: 1,
                },
                EntryVersion {
                    id: entries[1].id.clone(),
                    expected_revision: revision,
                },
            ]
        };
        assert_eq!(
            vault.delete_many(versions(99)).unwrap_err().code,
            "CONFLICT"
        );
        assert_eq!(vault.list("", None).unwrap().len(), 2);
        assert!(
            vault
                .update_many(vec![patch(&entries[0].id, 1), patch(&entries[0].id, 2)])
                .is_err()
        );
        let updated = vault
            .update_many(vec![patch(&entries[0].id, 1), patch(&entries[1].id, 1)])
            .unwrap();
        assert!(updated.iter().all(|v| v.revision == 2));
        vault.delete(&entries[0].id, 2).unwrap();
        assert_eq!(
            vault
                .delete_many(vec![EntryVersion {
                    id: entries[1].id.clone(),
                    expected_revision: 2
                }])
                .unwrap(),
            vec![entries[1].id.clone()]
        );
        drop(vault);
        let vault = Vault::open(dir.path()).unwrap();
        assert!(vault.list("", None).unwrap().is_empty());
        assert!(
            vault
                .protected_values()
                .unwrap()
                .iter()
                .any(|v| v == "batch-rotated-value")
        );
    }
    #[test]
    fn rotated_values_remain_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let e = vault.create(value()).unwrap();
        let patch:EntryPatch=serde_json::from_value(serde_json::json!({"id":e.id,"expectedRevision":1,"secret":"replacement-fixture-secret"})).unwrap();
        vault.update(patch).unwrap();
        drop(vault);
        let vault = Vault::open(dir.path()).unwrap();
        assert!(
            vault
                .protected_values()
                .unwrap()
                .iter()
                .any(|v| v == &value().secret)
        );
        vault.delete(&e.id, 2).unwrap();
        drop(vault);
        let vault = Vault::open(dir.path()).unwrap();
        assert!(
            vault
                .protected_values()
                .unwrap()
                .iter()
                .any(|v| v == "replacement-fixture-secret")
        );
    }
    fn value() -> EntryInput {
        EntryInput {
            group: String::new(),
            tags: vec![],
            database: None,
            kind: Kind::Api,
            title: "fixture-private-title".into(),
            account: "fixture-account".into(),
            url: "https://fixture.example.com".into(),
            services: Vec::new(),
            secret: "fixture-secret-DO-NOT-LEAK".into(),
            notes: "fixture-private-notes".into(),
        }
    }
    #[test]
    fn encrypted_roundtrip_conflicts_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let created = vault.create(value()).unwrap();
        assert!(
            !serde_json::to_string(&created)
                .unwrap()
                .contains("DO-NOT-LEAK")
        );
        assert_eq!(vault.get(&created.id).unwrap().secret, value().secret);
        let pinned = vault.pin(&created.id, 1, true).unwrap();
        assert_eq!(
            vault.pin(&created.id, 1, false).unwrap_err().code,
            "CONFLICT"
        );
        drop(vault);
        let vault = Vault::open(dir.path()).unwrap();
        assert!(vault.list("fixture", Some(Kind::Api)).unwrap()[0].pinned);
        assert!(vault.list("DO-NOT-LEAK", None).unwrap().is_empty());
        vault.delete(&created.id, pinned.revision).unwrap();
        assert!(vault.list("", None).unwrap().is_empty());
    }
    #[test]
    fn ciphertext_and_key_tampering_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let entry = vault.create(value()).unwrap();
        let conn = vault.connection().unwrap();
        conn.execute(
            "UPDATE entries SET encrypted=zeroblob(50) WHERE id=?1",
            [&entry.id],
        )
        .unwrap();
        assert_eq!(vault.get(&entry.id).err().unwrap().code, "DECRYPT_FAILED");
        conn.execute(
            "UPDATE metadata SET value=zeroblob(50) WHERE name='wrapped_key'",
            [],
        )
        .unwrap();
        drop(vault);
        assert_eq!(Vault::open(dir.path()).err().unwrap().code, "UNLOCK_FAILED");
    }
    #[test]
    fn no_plaintext_in_database_or_wal() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let e = value();
        vault.create(e.clone()).unwrap();
        for file in std::fs::read_dir(dir.path()).unwrap() {
            let bytes = std::fs::read(file.unwrap().path()).unwrap();
            for secret in [&e.title, &e.secret, &e.account, &e.url, &e.notes] {
                assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()));
            }
        }
    }
    #[test]
    fn unrelated_and_zero_length_databases_are_not_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        std::fs::write(&path, b"unrelated data").unwrap();
        assert!(Vault::open(dir.path()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"unrelated data");
        std::fs::write(&path, []).unwrap();
        assert!(Vault::open(dir.path()).is_err());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    }
    #[test]
    fn partial_update_preserves_secret_and_rejects_blank() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let e = vault.create(value()).unwrap();
        let patch: EntryPatch = serde_json::from_value(
            serde_json::json!({"id":e.id,"expectedRevision":1,"notes":"new-note"}),
        )
        .unwrap();
        vault.update(patch).unwrap();
        assert_eq!(vault.get(&e.id).unwrap().secret, value().secret);
        let patch: EntryPatch =
            serde_json::from_value(serde_json::json!({"id":e.id,"expectedRevision":2,"secret":""}))
                .unwrap();
        assert_eq!(vault.update(patch).err().unwrap().code, "INVALID_INPUT");
    }
}
