use super::*;

const CATALOG: &str = "organization-v1";
#[derive(Default, Serialize, Deserialize)]
pub struct Organization {
    pub tags: Vec<String>,
}

impl Vault {
    pub(super) fn organization_on(&self, c: &Connection) -> Result<Organization> {
        let packed: Option<Vec<u8>> = c
            .query_row("SELECT value FROM metadata WHERE name=?1", [CATALOG], |r| {
                r.get(0)
            })
            .optional()?;
        let mut catalog: Organization = match packed {
            Some(bytes) => {
                serde_json::from_slice(&crypto::decrypt(&self.key, CATALOG.as_bytes(), &bytes)?)
                    .map_err(|_| AppError::corrupt())?
            }
            None => Organization::default(),
        };
        let mut query = c.prepare(&format!(
            "SELECT {COLUMNS} FROM entries UNION ALL SELECT {COLUMNS} FROM trash"
        ))?;
        for row in query.query_map([], from_sql)? {
            catalog.tags.extend(self.decode(&row?)?.tags)
        }
        catalog.tags.sort();
        catalog.tags.dedup();
        Ok(catalog)
    }
    pub(super) fn save_catalog_on(&self, c: &Connection, catalog: &Organization) -> Result<()> {
        let plaintext =
            Zeroizing::new(serde_json::to_vec(catalog).map_err(|_| AppError::invalid())?);
        let packed = crypto::encrypt(&self.key, CATALOG.as_bytes(), &plaintext)?;
        c.execute("INSERT INTO metadata(name,value) VALUES (?1,?2) ON CONFLICT(name) DO UPDATE SET value=excluded.value",params![CATALOG,packed])?;
        Ok(())
    }
    pub fn organization(&self) -> Result<Organization> {
        self.organization_on(&self.connection()?)
    }
    pub fn organize(&self, kind: &str, name: &str, previous: Option<&str>) -> Result<()> {
        let name = name.trim();
        let deleting = name.is_empty() && previous.is_some();
        if kind != "tag" || (!deleting && name.is_empty()) || name.len() > 256 {
            return Err(AppError::invalid());
        }
        let mut c = self.connection()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut catalog = self.organization_on(&tx)?;
        if !deleting && catalog.tags.iter().any(|n| n == name) && previous != Some(name) {
            return Err(AppError::new("DUPLICATE_NAME", "标签名称已存在。"));
        }
        if let Some(old) = previous {
            if !catalog.tags.iter().any(|n| n == old) {
                return Err(AppError::conflict());
            }
            catalog.tags.retain(|n| n != old);
            for table in ["entries", "trash"] {
                let rows: Vec<Row> = {
                    let mut q = tx.prepare(&format!("SELECT {COLUMNS} FROM {table}"))?;
                    q.query_map([], from_sql)?
                        .collect::<std::result::Result<_, _>>()?
                };
                for mut row in rows {
                    let mut value = self.decode(&row)?;
                    if !value.tags.iter().any(|t| t == old) {
                        continue;
                    }
                    value.tags.retain(|t| t != old);
                    if !deleting && !value.tags.iter().any(|t| t == name) {
                        value.tags.push(name.into())
                    }
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
            let mut settings = self.settings_on(&tx)?;
            if let Some(rule) = settings.mcp_tag_rules.remove(old)
                && !deleting
            {
                settings.mcp_tag_rules.insert(name.into(), rule);
            }
            self.save_settings_on(&tx, &settings)?;
        }
        if !deleting {
            catalog.tags.push(name.into())
        }
        catalog.tags.sort();
        catalog.tags.dedup();
        self.save_catalog_on(&tx, &catalog)?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn database() -> EntryInput {
        serde_json::from_value(serde_json::json!({
            "kind":"database","title":"orders","tags":["orders-system","UVT"],
            "database":{"databaseType":"Oracle","host":"fixture.invalid","port":"1521","databaseName":"ORCL"},
            "secret":"","account":"fixture"
        })).unwrap()
    }
    #[test]
    fn database_organization_and_renames_persist_including_trash() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        vault.organize("tag", "empty-tag", None).unwrap();
        let a = vault.create(database()).unwrap();
        let b = vault.create(database()).unwrap();
        vault.delete(&b.id, b.revision).unwrap();
        vault
            .organize("tag", "renamed-system", Some("orders-system"))
            .unwrap();
        vault.organize("tag", "staging", Some("UVT")).unwrap();
        assert!(
            vault
                .get(&a.id)
                .unwrap()
                .summary
                .tags
                .contains(&"renamed-system".into())
        );
        assert!(vault.delete(&a.id, a.revision).is_err());
        let deleted = vault.list_trash().unwrap();
        assert!(deleted[0].summary.tags.contains(&"staging".into()));
        assert_eq!(
            deleted[0].summary.database.as_ref().unwrap().database_name,
            "ORCL"
        );
        assert_eq!(vault.list("fixture.invalid", None).unwrap().len(), 1);
        assert_eq!(vault.list("staging", None).unwrap().len(), 1);
        drop(vault);
        let reopened = Vault::open(dir.path()).unwrap();
        assert!(
            reopened
                .organization()
                .unwrap()
                .tags
                .contains(&"empty-tag".into())
        );
        assert_eq!(reopened.get(&a.id).unwrap().secret, "");
        let bytes = std::fs::read(dir.path().join("vault.db")).unwrap();
        assert!(
            !bytes
                .windows(b"renamed-system".len())
                .any(|w| w == b"renamed-system")
        );
    }
    #[test]
    fn invalid_database_fields_and_duplicate_names_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let mut value = database();
        value.database.as_mut().unwrap().port = "65536".into();
        assert!(vault.create(value).is_err());
        vault.organize("tag", "one", None).unwrap();
        vault.organize("tag", "two", None).unwrap();
        assert!(vault.organize("tag", "two", Some("one")).is_err());
        assert!(vault.organization().unwrap().tags.contains(&"one".into()));
    }
}
