use super::*;

const CATALOG: &str = "organization-v1";
#[derive(Default, Serialize, Deserialize)]
pub struct Organization {
    pub groups: Vec<String>,
    pub tags: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn database() -> EntryInput {
        serde_json::from_value(serde_json::json!({
            "kind":"database","title":"orders","group":"orders-system","tags":["UVT"],
            "database":{"databaseType":"Oracle","host":"fixture.invalid","port":"1521","databaseName":"ORCL"},
            "secret":"","account":"fixture"
        })).unwrap()
    }
    #[test]
    fn database_organization_and_renames_persist_including_trash() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        vault.organize("group", "empty-group", None).unwrap();
        let a = vault.create(database()).unwrap();
        let b = vault.create(database()).unwrap();
        vault.delete(&b.id, b.revision).unwrap();
        vault
            .organize("group", "renamed-system", Some("orders-system"))
            .unwrap();
        vault.organize("tag", "staging", Some("UVT")).unwrap();
        assert_eq!(vault.get(&a.id).unwrap().summary.group, "renamed-system");
        assert!(vault.delete(&a.id, a.revision).is_err());
        let deleted = vault.list_trash().unwrap();
        assert_eq!(deleted[0].summary.tags, vec!["staging"]);
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
                .groups
                .contains(&"empty-group".into())
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
        vault.organize("group", "one", None).unwrap();
        vault.organize("group", "two", None).unwrap();
        assert!(vault.organize("group", "two", Some("one")).is_err());
        assert!(vault.organization().unwrap().groups.contains(&"one".into()));
    }
}
impl Vault {
    fn organization_on(&self, connection: &Connection) -> Result<Organization> {
        let packed: Option<Vec<u8>> = connection
            .query_row("SELECT value FROM metadata WHERE name=?1", [CATALOG], |r| {
                r.get(0)
            })
            .optional()?;
        let mut catalog: Organization = match packed {
            Some(packed) => {
                serde_json::from_slice(&crypto::decrypt(&self.key, CATALOG.as_bytes(), &packed)?)
                    .map_err(|_| AppError::corrupt())?
            }
            None => Organization::default(),
        };
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS} FROM entries UNION ALL SELECT {COLUMNS} FROM trash"
        ))?;
        for row in statement.query_map([], from_sql)? {
            let value = self.decode(&row?)?;
            if !value.group.is_empty() {
                catalog.groups.push(value.group);
            }
            catalog.tags.extend(value.tags);
        }
        catalog.groups.sort();
        catalog.groups.dedup();
        catalog.tags.sort();
        catalog.tags.dedup();
        Ok(catalog)
    }
    pub fn organization(&self) -> Result<Organization> {
        self.organization_on(&self.connection()?)
    }
    pub fn organize(&self, kind: &str, name: &str, previous: Option<&str>) -> Result<()> {
        let name = name.trim();
        if !["group", "tag"].contains(&kind) || name.is_empty() || name.len() > 128 {
            return Err(AppError::invalid());
        }
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut catalog = self.organization_on(&tx)?;
        let names = if kind == "group" {
            &mut catalog.groups
        } else {
            &mut catalog.tags
        };
        if names.iter().any(|n| n == name) && previous != Some(name) {
            return Err(AppError::new("DUPLICATE_NAME", "名称已存在。"));
        }
        if let Some(old) = previous {
            if !names.iter().any(|n| n == old) {
                return Err(AppError::conflict());
            }
            names.retain(|n| n != old);
            // Rename active and deleted entries atomically; all old revisions become stale.
            for table in ["entries", "trash"] {
                let rows: Vec<Row> = {
                    let mut statement = tx.prepare(&format!("SELECT {COLUMNS} FROM {table}"))?;
                    statement
                        .query_map([], from_sql)?
                        .collect::<std::result::Result<_, _>>()?
                };
                for mut row in rows {
                    let mut value = self.decode(&row)?;
                    let changed = if kind == "group" {
                        if value.group == old {
                            value.group = name.into();
                            true
                        } else {
                            false
                        }
                    } else if value.tags.iter().any(|t| t == old) {
                        value.tags.iter_mut().for_each(|t| {
                            if t == old {
                                *t = name.into();
                            }
                        });
                        value.tags.sort();
                        value.tags.dedup();
                        true
                    } else {
                        false
                    };
                    if changed {
                        row.revision += 1;
                        row.updated_at = now();
                        tx.execute(&format!("UPDATE {table} SET encrypted=?1,revision=?2,updated_at=?3 WHERE id=?4"),
                            params![self.encode(&row, &value)?, row.revision, row.updated_at, row.id])?;
                    }
                }
            }
        }
        names.push(name.into());
        let plaintext =
            Zeroizing::new(serde_json::to_vec(&catalog).map_err(|_| AppError::invalid())?);
        let encrypted = crypto::encrypt(&self.key, CATALOG.as_bytes(), &plaintext)?;
        tx.execute("INSERT INTO metadata(name,value) VALUES (?1,?2) ON CONFLICT(name) DO UPDATE SET value=excluded.value", params![CATALOG, encrypted])?;
        tx.commit()?;
        Ok(())
    }
}
