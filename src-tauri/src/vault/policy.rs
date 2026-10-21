use super::*;
use std::collections::BTreeMap;
const POLICY: &str = "mcp-object-policy-v1";
type Rules = (
    BTreeMap<String, PermissionRule>,
    BTreeMap<String, PermissionRule>,
);
impl Vault {
    pub(super) fn settings_on(&self, c: &Connection) -> Result<Settings> {
        let bytes: Vec<u8> = c.query_row(
            "SELECT value FROM metadata WHERE name='settings'",
            [],
            |r| r.get(0),
        )?;
        let mut s: Settings = serde_json::from_slice(&bytes).map_err(|_| AppError::corrupt())?;
        let packed: Option<Vec<u8>> = c
            .query_row("SELECT value FROM metadata WHERE name=?1", [POLICY], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(packed) = packed {
            let rules: Rules =
                serde_json::from_slice(&crypto::decrypt(&self.key, POLICY.as_bytes(), &packed)?)
                    .map_err(|_| AppError::corrupt())?;
            s.mcp_tag_rules = rules.0;
            s.mcp_entry_rules = rules.1;
        }
        Ok(s)
    }
    pub(super) fn save_settings_on(&self, c: &Connection, s: &Settings) -> Result<()> {
        if s.mcp_tag_rules.len() > 2000
            || s.mcp_entry_rules.len() > 10000
            || s.mcp_tag_rules
                .keys()
                .any(|t| t.trim() != t || t.is_empty() || t.len() > 256)
            || s.mcp_entry_rules
                .keys()
                .any(|id| uuid::Uuid::parse_str(id).is_err())
        {
            return Err(AppError::invalid());
        }
        let mut base = s.clone();
        base.mcp_tag_rules.clear();
        base.mcp_entry_rules.clear();
        let bytes = serde_json::to_vec(&base).map_err(|_| AppError::invalid())?;
        let rules = Zeroizing::new(
            serde_json::to_vec(&(&s.mcp_tag_rules, &s.mcp_entry_rules))
                .map_err(|_| AppError::invalid())?,
        );
        let packed = crypto::encrypt(&self.key, POLICY.as_bytes(), &rules)?;
        c.execute(
            "UPDATE metadata SET value=?1 WHERE name='settings'",
            [bytes],
        )?;
        c.execute("INSERT INTO metadata(name,value) VALUES (?1,?2) ON CONFLICT(name) DO UPDATE SET value=excluded.value",params![POLICY,packed])?;
        Ok(())
    }
    pub(super) fn normalize_labels(value: &mut EntryInput) {
        if !value.group.trim().is_empty() {
            value.tags.push(value.group.trim().into());
            value.group.clear();
        }
        value.tags = value
            .tags
            .iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        let mut seen = std::collections::HashSet::new();
        value.tags.retain(|t| seen.insert(t.clone()));
    }
    pub(super) fn migrate_labels(&self) -> Result<()> {
        let mut c = self.connection()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Idempotent, encrypted, lossless migration. Also handles a legacy process
        // writing an old record after the initial upgrade.
        for table in ["entries", "trash"] {
            let rows: Vec<Row> = {
                let mut q = tx.prepare(&format!("SELECT {COLUMNS} FROM {table}"))?;
                q.query_map([], from_sql)?
                    .collect::<std::result::Result<_, _>>()?
            };
            for mut row in rows {
                let plaintext = crypto::decrypt(&self.key, &row.aad(), &row.packed)?;
                let mut value: EntryInput =
                    serde_json::from_slice(&plaintext).map_err(|_| AppError::corrupt())?;
                if value.group.trim().is_empty() {
                    continue;
                }
                Self::normalize_labels(&mut value);
                row.revision += 1;
                row.updated_at = now();
                tx.execute(
                    &format!(
                        "UPDATE {table} SET encrypted=?1,revision=?2,updated_at=?3 WHERE id=?4"
                    ),
                    params![
                        self.encode(&row, &value)?,
                        row.revision,
                        row.updated_at,
                        row.id
                    ],
                )?;
            }
        }
        let mut catalog = self.organization_on(&tx)?;
        catalog.tags.append(&mut catalog.groups);
        catalog.tags.sort();
        catalog.tags.dedup();
        self.save_catalog_on(&tx, &catalog)?;
        tx.execute_batch("PRAGMA user_version=3;")?;
        tx.commit()?;
        Ok(())
    }
}
