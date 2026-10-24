//! Desktop recovery operations. Intentionally absent from the MCP dispatcher.
use super::*;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashEntry {
    #[serde(flatten)]
    pub summary: EntrySummary,
    pub deleted_at: i64,
}

impl Vault {
    pub fn list_trash(&self) -> Result<Vec<TrashEntry>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS},deleted_at FROM trash ORDER BY deleted_at DESC,id"
        ))?;
        let rows = statement.query_map([], |r| Ok((from_sql(r)?, r.get::<_, i64>(6)?)))?;
        rows.map(|record| {
            let (row, deleted_at) = record?;
            let value = self.decode(&row)?;
            Ok(TrashEntry {
                summary: row.summary(&value),
                deleted_at,
            })
        })
        .collect()
    }

    pub fn restore(&self, id: &str, expected_revision: i64) -> Result<EntrySummary> {
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut row = tx
            .query_row(
                &format!("SELECT {COLUMNS} FROM trash WHERE id=?1"),
                [id],
                from_sql,
            )
            .optional()?
            .ok_or(AppError::new("NOT_FOUND", "垃圾桶条目不存在。"))?;
        if row.revision != expected_revision {
            return Err(AppError::conflict());
        }
        let value = self.decode(&row)?;
        // Old clients must not overwrite a restored entry using a pre-delete version.
        row.revision += 1;
        row.updated_at = now();
        let packed = self.encode(&row, &value)?;
        tx.execute("INSERT INTO entries (id,encrypted,revision,pinned,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![row.id, packed, row.revision, row.pinned, row.created_at, row.updated_at])?;
        tx.execute("DELETE FROM trash WHERE id=?1", [id])?;
        tx.commit()?;
        Ok(row.summary(&value))
    }

    /// Delete only the exact snapshot the human confirmed; concurrent changes fail.
    pub fn empty_trash(&self, expected: Vec<EntryVersion>) -> Result<usize> {
        let mut connection = self.connection()?;
        connection.execute_batch("PRAGMA secure_delete=ON;")?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut actual: Vec<(String, i64)> = {
            let mut statement = tx.prepare("SELECT id,revision FROM trash")?;
            statement
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<std::result::Result<_, _>>()?
        };
        let mut expected: Vec<_> = expected
            .into_iter()
            .map(|e| (e.id, e.expected_revision))
            .collect();
        actual.sort();
        expected.sort();
        if actual != expected {
            return Err(AppError::conflict());
        }
        // Retired values use entry-owned keys in the current format.
        for (id, _) in &actual {
            tx.execute("DELETE FROM metadata WHERE name GLOB ?1", [format!("retired_secret:{id}:*")])?;
        }
        let count = tx.execute("DELETE FROM trash", [])?;
        tx.commit()?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> EntryInput {
        serde_json::from_value(serde_json::json!({
            "kind":"password","title":"trash fixture","secret":"trash-fixture-secret",
            "account":"fixture-user","notes":"retained note"
        }))
        .unwrap()
    }
    #[test]
    fn soft_delete_blocks_normal_access_and_restores_with_new_revision() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let entry = vault.create(input()).unwrap();
        vault.delete(&entry.id, entry.revision).unwrap();
        assert!(vault.get(&entry.id).is_err());
        assert!(vault.list("", None).unwrap().is_empty());
        assert!(vault.pin(&entry.id, entry.revision, true).is_err());
        assert!(
            vault
                .protected_values()
                .unwrap()
                .iter()
                .any(|s| s == "trash-fixture-secret")
        );
        drop(vault);
        let vault = Vault::open(dir.path()).unwrap();
        let trash = vault.list_trash().unwrap();
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].summary.notes, "retained note");
        let restored = vault.restore(&entry.id, entry.revision).unwrap();
        assert!(restored.revision > entry.revision);
        assert_eq!(vault.get(&entry.id).unwrap().secret, "trash-fixture-secret");
        assert!(vault.delete(&entry.id, entry.revision).is_err());
        assert!(vault.list_trash().unwrap().is_empty());
    }

    #[test]
    fn purge_requires_exact_snapshot_and_leaves_active_entries() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let first = vault.create(input()).unwrap();
        let active = vault.create(input()).unwrap();
        vault.delete(&first.id, 1).unwrap();
        assert!(vault.empty_trash(vec![]).is_err());
        let count = vault
            .empty_trash(vec![EntryVersion {
                id: first.id.clone(),
                expected_revision: 1,
            }])
            .unwrap();
        assert_eq!(count, 1);
        assert!(vault.restore(&first.id, 1).is_err());
        assert!(vault.get(&active.id).is_ok());
    }
    #[test]
    fn purge_removes_owned_retired_secret_material() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let entry = vault.create(input()).unwrap();
        let patch: EntryPatch = serde_json::from_value(serde_json::json!({
            "id":entry.id,"expectedRevision":1,"secret":"rotated-trash-fixture"
        }))
        .unwrap();
        vault.update(patch).unwrap();
        vault.delete(&entry.id, 2).unwrap();
        vault
            .empty_trash(vec![EntryVersion {
                id: entry.id,
                expected_revision: 2,
            }])
            .unwrap();
        assert!(vault.protected_values().unwrap().is_empty());
    }
}
