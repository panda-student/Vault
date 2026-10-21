use super::*;
#[test]
fn v2_groups_migrate_once_including_trash_and_backup_preserves_encrypted_original() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path()).unwrap();
    let input:EntryInput=serde_json::from_value(serde_json::json!({"kind":"password","title":"legacy","account":"legacy-user","url":"https://fixture.invalid","secret":"FAKE_MIGRATION_SECRET_6341","notes":"keep notes","tags":["existing"]})).unwrap();
    let a = vault.create(input.clone()).unwrap();
    let b = vault.create(input.clone()).unwrap();
    vault.delete(&b.id, b.revision).unwrap();
    let c = vault.connection().unwrap();
    for (table, id) in [("entries", &a.id), ("trash", &b.id)] {
        let row: Row = c
            .query_row(
                &format!("SELECT {COLUMNS} FROM {table} WHERE id=?1"),
                [id],
                from_sql,
            )
            .unwrap();
        let mut old = input.clone();
        old.group = "LEGACY_SYSTEM_TAG".into();
        old.tags.push("LEGACY_SYSTEM_TAG".into());
        c.execute(
            &format!("UPDATE {table} SET encrypted=?1 WHERE id=?2"),
            params![vault.encode(&row, &old).unwrap(), id],
        )
        .unwrap();
    }
    let cat = Organization {
        groups: vec!["EMPTY_LEGACY_GROUP".into()],
        tags: vec![],
    };
    vault.save_catalog_on(&c, &cat).unwrap();
    let legacy = serde_json::json!({"mcpEnabled":true,"closeToTray":true,"mcpPermissions":{"readMetadata":true,"create":true,"update":false,"delete":true,"fillInput":true,"writeFile":false}});
    c.execute(
        "UPDATE metadata SET value=?1 WHERE name='settings'",
        [serde_json::to_vec(&legacy).unwrap()],
    )
    .unwrap();
    c.execute_batch("PRAGMA user_version=2;").unwrap();
    drop(c);
    drop(vault);
    let vault = Vault::open(dir.path()).unwrap();
    let current = vault.get(&a.id).unwrap();
    assert!(current.summary.group.is_empty());
    assert_eq!(current.summary.tags, vec!["existing", "LEGACY_SYSTEM_TAG"]);
    assert_eq!(current.secret, input.secret);
    assert_eq!(current.summary.notes, input.notes);
    assert_eq!(current.summary.revision, a.revision + 1);
    let trash = vault.list_trash().unwrap();
    assert!(trash[0].summary.tags.contains(&"LEGACY_SYSTEM_TAG".into()));
    let cat = vault.organization().unwrap();
    assert!(cat.groups.is_empty());
    assert!(cat.tags.contains(&"EMPTY_LEGACY_GROUP".into()));
    assert!(!vault.settings().unwrap().mcp_permissions.write_file);
    assert!(!vault.settings().unwrap().mcp_permissions.update);
    assert!(vault.settings().unwrap().close_to_tray);
    assert_eq!(
        std::fs::read_dir(dir.path().join("backups"))
            .unwrap()
            .count(),
        1
    );
    let revision = current.summary.revision;
    drop(vault);
    let reopened = Vault::open(dir.path()).unwrap();
    assert_eq!(reopened.get(&a.id).unwrap().summary.revision, revision);
    assert_eq!(
        std::fs::read_dir(dir.path().join("backups"))
            .unwrap()
            .count(),
        1
    );
}
