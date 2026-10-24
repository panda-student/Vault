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
        value.tags = value
            .tags
            .iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        let mut seen = std::collections::HashSet::new();
        value.tags.retain(|t| seen.insert(t.clone()));
    }
}
