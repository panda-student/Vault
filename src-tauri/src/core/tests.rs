use super::*;
use serde_json::json;
fn input(title: &str) -> EntryInput {
    serde_json::from_value(
        json!({"kind":"password","title":title,"secret":"FAKE_CORE_TEST_SECRET_8192"}),
    )
    .unwrap()
}
fn tagged(title: &str, tags: &[&str]) -> EntryInput {
    let mut v = input(title);
    v.tags = tags.iter().map(|t| t.to_string()).collect();
    v
}
#[test]
fn tag_rules_filter_search_and_batch_read_and_entry_override_wins() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = open(dir.path(), false).unwrap();
    let mcp = open(dir.path(), true).unwrap();
    let es: Vec<EntrySummary> = desktop
        .call(Operation::Create(vec![
            tagged("visible", &["work"]),
            tagged("hidden", &["work", "production"]),
            tagged("plain", &[]),
        ]))
        .unwrap();
    let mut s = settings(&desktop);
    s.mcp_access = false;
    s.mcp_tag_rules.insert(
        "work".into(),
        serde_json::from_value(json!({"access":true})).unwrap(),
    );
    s.mcp_tag_rules.insert(
        "production".into(),
        serde_json::from_value(json!({"access":false})).unwrap(),
    );
    desktop.execute(Operation::SaveSettings(s.clone())).unwrap();
    let result = mcp
        .execute(Operation::Search(Search::new("".into(), None)))
        .unwrap();
    assert_eq!(result["entries"].as_array().unwrap().len(), 1);
    assert_eq!(result["entries"][0]["id"], es[0].id);
    assert_eq!(
        mcp.execute(Operation::Read(vec![es[0].id.clone(), es[1].id.clone()]))
            .unwrap_err()
            .code,
        "PERMISSION_DENIED"
    );
    s.mcp_entry_rules.insert(
        es[1].id.clone(),
        serde_json::from_value(json!({"access":true})).unwrap(),
    );
    desktop.execute(Operation::SaveSettings(s)).unwrap();
    assert!(mcp.execute(Operation::Read(vec![es[1].id.clone()])).is_ok());
    assert!(mcp.tool_allowed("get_entries").unwrap());
}
#[test]
fn scoped_batch_mutations_roll_back_and_mcp_cannot_relabel_to_escalate() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = open(dir.path(), false).unwrap();
    let mcp = open(dir.path(), true).unwrap();
    let mut s = settings(&desktop);
    s.mcp_tag_rules.insert(
        "restricted".into(),
        serde_json::from_value(json!({"create":false,"update":false,"delete":false})).unwrap(),
    );
    s.mcp_tag_rules.insert(
        "production".into(),
        serde_json::from_value(json!({"writeFile":false})).unwrap(),
    );
    desktop.execute(Operation::SaveSettings(s)).unwrap();
    assert!(
        mcp.execute(Operation::Create(vec![
            tagged("would save", &[]),
            tagged("denied", &["restricted"])
        ]))
        .is_err()
    );
    assert!(desktop.service.storage.list("", None).unwrap().is_empty());
    let es: Vec<EntrySummary> = desktop
        .call(Operation::Create(vec![
            tagged("first", &[]),
            tagged("blocked", &["restricted"]),
            tagged("prod", &["production"]),
        ]))
        .unwrap();
    let patches = es[..2]
        .iter()
        .map(|e| {
            serde_json::from_value(
                json!({"id":e.id,"expectedRevision":e.revision,"title":"changed"}),
            )
            .unwrap()
        })
        .collect();
    assert!(mcp.execute(Operation::Update(patches)).is_err());
    assert_eq!(
        desktop.service.storage.summary(&es[0].id).unwrap().title,
        "first"
    );
    let deletes = es[..2]
        .iter()
        .map(|e| EntryVersion {
            id: e.id.clone(),
            expected_revision: e.revision,
        })
        .collect();
    assert!(mcp.execute(Operation::Delete(deletes)).is_err());
    assert!(desktop.service.storage.list_trash().unwrap().is_empty());
    let patch =
        serde_json::from_value(json!({"id":es[2].id,"expectedRevision":es[2].revision,"tags":[]}))
            .unwrap();
    assert_eq!(
        mcp.execute(Operation::Update(vec![patch]))
            .unwrap_err()
            .code,
        "PERMISSION_DENIED"
    );
    let target = dir.path().join("must-not-exist.txt");
    assert!(mcp.execute(write(&es[2].id, &target)).is_err());
    assert!(!target.exists());
}
#[test]
fn rules_rename_delete_encrypt_and_remain_attached_to_restored_entry() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = open(dir.path(), false).unwrap();
    let mcp = open(dir.path(), true).unwrap();
    let es: Vec<EntrySummary> = desktop
        .call(Operation::Create(vec![
            tagged("one", &["CONFIDENTIAL_TAG_4821"]),
            tagged("two", &["CONFIDENTIAL_TAG_4821"]),
        ]))
        .unwrap();
    let mut s = settings(&desktop);
    s.mcp_tag_rules.insert(
        "CONFIDENTIAL_TAG_4821".into(),
        serde_json::from_value(json!({"writeFile":false})).unwrap(),
    );
    s.mcp_entry_rules.insert(
        es[0].id.clone(),
        serde_json::from_value(json!({"readMetadata":false})).unwrap(),
    );
    desktop.execute(Operation::SaveSettings(s)).unwrap();
    desktop
        .execute(Operation::Delete(vec![EntryVersion {
            id: es[1].id.clone(),
            expected_revision: es[1].revision,
        }]))
        .unwrap();
    desktop
        .execute(Operation::Organize {
            kind: "tag".into(),
            name: "RENAMED_TAG".into(),
            previous: Some("CONFIDENTIAL_TAG_4821".into()),
        })
        .unwrap();
    assert!(settings(&desktop).mcp_tag_rules.contains_key("RENAMED_TAG"));
    assert!(
        !settings(&desktop)
            .mcp_tag_rules
            .contains_key("CONFIDENTIAL_TAG_4821")
    );
    let trashed = desktop.service.storage.list_trash().unwrap().remove(0);
    let restored: EntrySummary = desktop
        .call(Operation::Restore {
            id: trashed.summary.id,
            expected_revision: trashed.summary.revision,
        })
        .unwrap();
    assert_eq!(restored.tags, vec!["RENAMED_TAG"]);
    assert!(
        mcp.execute(write(&restored.id, &dir.path().join("denied.txt")))
            .is_err()
    );
    assert_eq!(
        mcp.execute(Operation::Read(vec![es[0].id.clone()]))
            .unwrap_err()
            .code,
        "PERMISSION_DENIED"
    );
    for file in ["vault.db", "vault.db-wal"] {
        if let Ok(bytes) = std::fs::read(dir.path().join(file)) {
            assert!(
                !bytes
                    .windows(b"CONFIDENTIAL_TAG_4821".len())
                    .any(|w| w == b"CONFIDENTIAL_TAG_4821")
            );
        }
    }
    desktop
        .execute(Operation::Organize {
            kind: "tag".into(),
            name: "".into(),
            previous: Some("RENAMED_TAG".into()),
        })
        .unwrap();
    assert!(settings(&desktop).mcp_tag_rules.is_empty());
    assert!(
        desktop
            .service
            .storage
            .summary(&restored.id)
            .unwrap()
            .tags
            .is_empty()
    );
}
#[test]
fn preview_uses_core_sources_and_is_never_available_to_mcp() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = open(dir.path(), false).unwrap();
    let mcp = open(dir.path(), true).unwrap();
    let e: Vec<EntrySummary> = desktop
        .call(Operation::Create(vec![tagged("preview", &["yes", "no"])]))
        .unwrap();
    let mut s = settings(&desktop);
    s.mcp_tag_rules.insert(
        "yes".into(),
        serde_json::from_value(json!({"writeFile":true})).unwrap(),
    );
    s.mcp_tag_rules.insert(
        "no".into(),
        serde_json::from_value(json!({"writeFile":false})).unwrap(),
    );
    let r = desktop
        .execute(Operation::PermissionPreview(s.clone()))
        .unwrap();
    assert_eq!(r[0]["permissions"]["writeFile"]["allowed"], false);
    assert!(
        r[0]["permissions"]["writeFile"]["source"]
            .as_str()
            .unwrap()
            .contains("禁止优先")
    );
    assert!(
        mcp.execute(Operation::PermissionPreview(s.clone()))
            .is_err()
    );
    s.mcp_entry_rules.insert(
        e[0].id.clone(),
        serde_json::from_value(json!({"writeFile":true})).unwrap(),
    );
    assert_eq!(
        desktop.execute(Operation::PermissionPreview(s)).unwrap()[0]["permissions"]["writeFile"]["source"],
        "单条设置"
    );
}
fn create(session: &CoreSession, title: &str) -> EntrySummary {
    session
        .call::<Vec<EntrySummary>>(Operation::Create(vec![input(title)]))
        .unwrap()
        .remove(0)
}
fn field(id: &str, revision: i64) -> Operation {
    Operation::ReadField(FieldRequest {
        id: id.into(),
        expected_revision: revision,
        field: "secret".into(),
        service_id: None,
    })
}
fn settings(session: &CoreSession) -> Settings {
    session.call(Operation::Settings).unwrap()
}
#[test]
fn mcp_can_delete_catalog_only_tag_without_touching_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = open(dir.path(), false).unwrap();
    let mcp = open(dir.path(), true).unwrap();
    let entries: Vec<EntrySummary> = desktop
        .call(Operation::Create(vec![tagged("keep", &["keep-tag"])]))
        .unwrap();
    desktop
        .execute(Operation::Organize {
            kind: "tag".into(),
            name: "empty-tag".into(),
            previous: None,
        })
        .unwrap();
    let mut rules = settings(&desktop);
    rules.mcp_permissions.update = false;
    desktop
        .execute(Operation::SaveSettings(rules.clone()))
        .unwrap();
    assert_eq!(
        mcp.execute(Operation::Organize {
            kind: "tag".into(),
            name: String::new(),
            previous: Some("empty-tag".into()),
        })
        .unwrap_err()
        .code,
        "PERMISSION_DENIED"
    );
    let org: Organization = desktop.call(Operation::Organization).unwrap();
    assert!(org.tags.contains(&"empty-tag".into()));
    rules.mcp_permissions.update = true;
    rules.mcp_tag_rules.insert(
        "empty-tag".into(),
        serde_json::from_value(json!({"delete":false})).unwrap(),
    );
    rules.mcp_tag_rules.insert(
        "keep-tag".into(),
        serde_json::from_value(json!({"writeFile":false})).unwrap(),
    );
    desktop.execute(Operation::SaveSettings(rules)).unwrap();
    mcp.execute(Operation::Organize {
        kind: "tag".into(),
        name: String::new(),
        previous: Some("empty-tag".into()),
    })
    .unwrap();
    let org: Organization = desktop.call(Operation::Organization).unwrap();
    assert_eq!(org.tags, vec!["keep-tag"]);
    let rules = settings(&desktop);
    assert!(!rules.mcp_tag_rules.contains_key("empty-tag"));
    assert!(rules.mcp_tag_rules.contains_key("keep-tag"));
    let unchanged = desktop.service.storage.summary(&entries[0].id).unwrap();
    assert_eq!(unchanged.revision, entries[0].revision);
    assert_eq!(unchanged.tags, entries[0].tags);
    assert_eq!(
        mcp.execute(Operation::Organize {
            kind: "tag".into(),
            name: String::new(),
            previous: Some("empty-tag".into()),
        })
        .unwrap_err()
        .code,
        "NOT_FOUND"
    );
    drop(mcp);
    drop(desktop);
    let reopened = open(dir.path(), false).unwrap();
    let org: Organization = reopened.call(Operation::Organization).unwrap();
    assert_eq!(org.tags, vec!["keep-tag"]);
}
#[test]
fn mcp_restore_pin_and_organize_stay_within_update_permission() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = open(dir.path(), false).unwrap();
    let mcp = open(dir.path(), true).unwrap();
    // 桌面端创建带备注的条目并移入垃圾桶；MCP 恢复与置顶只应拿到受控元数据（无备注、无密钥）。
    let es: Vec<EntrySummary> = desktop
        .call(Operation::Create(vec![tagged("trashed", &["vault-work"])]))
        .unwrap();
    desktop
        .execute(Operation::Delete(vec![EntryVersion {
            id: es[0].id.clone(),
            expected_revision: es[0].revision,
        }]))
        .unwrap();
    let restored = mcp
        .execute(Operation::Restore {
            id: es[0].id.clone(),
            expected_revision: es[0].revision,
        })
        .unwrap();
    assert!(restored.get("notes").is_none());
    assert!(restored.get("secret").is_none());
    assert_eq!(restored["revision"].as_i64().unwrap(), es[0].revision + 1);
    let pinned = mcp
        .execute(Operation::Pin {
            id: es[0].id.clone(),
            expected_revision: es[0].revision + 1,
            pinned: true,
        })
        .unwrap();
    assert!(pinned.get("notes").is_none());
    assert_eq!(pinned["pinned"], json!(true));
    // MCP 重命名标签：标签上的禁止规则随重命名迁移，权限不变时应当允许。
    let mut s = settings(&desktop);
    s.mcp_tag_rules.insert(
        "vault-work".into(),
        serde_json::from_value(json!({"delete":false})).unwrap(),
    );
    desktop.execute(Operation::SaveSettings(s)).unwrap();
    mcp.execute(Operation::Organize {
        kind: "tag".into(),
        name: "vault-team".into(),
        previous: Some("vault-work".into()),
    })
    .unwrap();
    assert!(
        desktop
            .service
            .storage
            .summary(&es[0].id)
            .unwrap()
            .tags
            .contains(&"vault-team".into())
    );
    assert!(settings(&desktop).mcp_tag_rules.contains_key("vault-team"));
    // 删除带禁止规则的标签会让凭据回落到更宽松的默认规则，构成提权，必须拒绝。
    assert_eq!(
        mcp.execute(Operation::Organize {
            kind: "tag".into(),
            name: String::new(),
            previous: Some("vault-team".into()),
        })
        .unwrap_err()
        .code,
        "PERMISSION_DENIED"
    );
}
#[test]
fn validation_errors_carry_field_details_for_self_correction() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = open(dir.path(), false).unwrap();
    let mcp = open(dir.path(), true).unwrap();
    let es: Vec<EntrySummary> = desktop
        .call(Operation::Create(vec![tagged("detail target", &[])]))
        .unwrap();
    // 复现本次排查的契约冲突：非 API 凭据同时携带 services 与 url，
    // 错误必须点名 services 字段并说明规则，AI 才能据此自纠而不是盲试。
    let patch: EntryPatch = serde_json::from_value(json!({
        "id":es[0].id,"expectedRevision":es[0].revision,
        "url":"https://example.invalid","services":[]
    }))
    .unwrap();
    let error = mcp.execute(Operation::Update(vec![patch])).unwrap_err();
    assert_eq!(error.code, "INVALID_INPUT");
    let details = error.details.as_ref().unwrap();
    assert_eq!(details[0].field, "services");
    assert!(details[0].reason.contains("url"));
    // 空名称创建失败时错误必须点名 title 字段。
    let error = mcp
        .execute(Operation::Create(vec![input("   ")]))
        .unwrap_err();
    assert_eq!(error.details.as_ref().unwrap()[0].field, "title");
}

fn write(id: &str, path: &Path) -> Operation {
    Operation::Write(
        serde_json::from_value(
            json!({"credentialId":id,"field":"secret","target":{"type":"file","path":path}}),
        )
        .unwrap(),
    )
}
#[test]
fn shared_interface_preserves_revision_trash_and_local_writes() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = open(dir.path(), false).unwrap();
    let mcp = open(dir.path(), true).unwrap();
    let entry = create(&desktop, "shared service");
    let patch = serde_json::from_value(
        json!({"id":entry.id,"expectedRevision":entry.revision,"account":"updated"}),
    )
    .unwrap();
    let updated = mcp.execute(Operation::Update(vec![patch])).unwrap();
    let revision = updated[0]["revision"].as_i64().unwrap();
    assert_eq!(
        desktop
            .execute(field(&entry.id, entry.revision))
            .unwrap_err()
            .code,
        "CONFLICT"
    );
    assert_eq!(
        desktop
            .execute(Operation::Read(vec![entry.id.clone()]))
            .unwrap()[0]["account"],
        "updated"
    );
    let target = dir.path().join("credential.txt");
    assert!(
        !mcp.execute(write(&entry.id, &target))
            .unwrap()
            .to_string()
            .contains("FAKE_CORE_TEST_SECRET")
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "FAKE_CORE_TEST_SECRET_8192"
    );
    mcp.execute(Operation::Delete(vec![EntryVersion {
        id: entry.id.clone(),
        expected_revision: revision,
    }]))
    .unwrap();
    assert!(
        desktop
            .execute(Operation::Read(vec![entry.id.clone()]))
            .is_err()
    );
    assert!(
        mcp.execute(write(&entry.id, &dir.path().join("deleted.txt")))
            .is_err()
    );
    let trashed = desktop
        .call::<Vec<TrashEntry>>(Operation::ListTrash)
        .unwrap()
        .remove(0);
    let restored: EntrySummary = desktop
        .call(Operation::Restore {
            id: entry.id.clone(),
            expected_revision: trashed.summary.revision,
        })
        .unwrap();
    assert_eq!(
        desktop
            .execute(field(&entry.id, restored.revision))
            .unwrap(),
        "FAKE_CORE_TEST_SECRET_8192"
    );
}
#[test]
fn shared_read_operation_projects_by_bound_caller_and_redacts_before_search() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = open(dir.path(), false).unwrap();
    let mcp = open(dir.path(), true).unwrap();
    let mut value = input("FAKE_CORE_TEST_SECRET_8192");
    value.notes = "private note".into();
    let entry = desktop
        .call::<Vec<EntrySummary>>(Operation::Create(vec![value]))
        .unwrap()
        .remove(0);
    let metadata = mcp
        .execute(Operation::Read(vec![entry.id.clone()]))
        .unwrap();
    assert!(metadata[0]["title"].is_null());
    assert!(metadata[0].get("notes").is_none());
    assert!(metadata[0].get("secret").is_none());
    assert_eq!(
        desktop.execute(Operation::Read(vec![entry.id])).unwrap()[0]["notes"],
        "private note"
    );
    let found = mcp
        .execute(Operation::Search(Search::new(
            "FAKE_CORE_TEST_SECRET_8192".into(),
            None,
        )))
        .unwrap();
    assert!(found["entries"].as_array().unwrap().is_empty());
}
#[test]
fn core_denies_all_desktop_only_operations_even_if_mcp_adapter_misroutes() {
    let dir = tempfile::tempdir().unwrap();
    let mcp = open(dir.path(), true).unwrap();
    let operations = vec![
        field("missing", 1),
        Operation::CopyText("forbidden".into()),
        Operation::CopyField(FieldRequest {
            id: "missing".into(),
            expected_revision: 1,
            field: "secret".into(),
            service_id: None,
        }),
        Operation::Settings,
        Operation::SaveSettings(Settings::default()),
        Operation::ListTrash,
        // Restore/Pin/Organize 已按 P2 设计开放为 MCP 的 Update 权限操作，
        // 不再属于 desktop-only 集合，其放行与提权防护由
        // mcp_restore_pin_and_organize_stay_within_update_permission 单独覆盖。
        Operation::EmptyTrash { owner_window: 0 },
        Operation::Organization,
        Operation::StartupEnabled(dir.path().join("fake.exe")),
        Operation::SetStartup {
            executable: dir.path().join("fake.exe"),
            enabled: true,
        },
    ];
    for op in operations {
        assert_eq!(mcp.execute(op).unwrap_err().code, "PERMISSION_DENIED");
    }
}
#[test]
fn six_permissions_revoke_live_sessions_and_do_not_restrict_desktop() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = open(dir.path(), false).unwrap();
    let mcp = open(dir.path(), true).unwrap();
    let entry = create(&desktop, "permission fixture");
    let mut policy = settings(&desktop);
    policy.mcp_permissions = crate::vault::McpPermissions {
        read_metadata: false,
        create: false,
        update: false,
        delete: false,
        fill_input: false,
        write_file: false,
    };
    desktop.execute(Operation::SaveSettings(policy)).unwrap();
    let patch = serde_json::from_value(
        json!({"id":entry.id,"expectedRevision":entry.revision,"title":"denied"}),
    )
    .unwrap();
    let operations = vec![
        Operation::Read(vec![entry.id.clone()]),
        Operation::Search(Search::new("".into(), None)),
        Operation::Create(vec![input("denied")]),
        Operation::Update(vec![patch]),
        Operation::Delete(vec![EntryVersion {
            id: entry.id.clone(),
            expected_revision: entry.revision,
        }]),
        Operation::CaptureTarget(CaptureRequest::default()),
        Operation::Fill(FillRequest {
            credential_id: entry.id.clone(),
            field: "secret".into(),
            service_id: None,
            target_id: "unused".into(),
        }),
        write(&entry.id, &dir.path().join("denied.txt")),
    ];
    for op in operations {
        assert_eq!(mcp.execute(op).unwrap_err().code, "PERMISSION_DENIED");
    }
    for name in [
        "get_entries",
        "search_entries",
        "create_entries",
        "update_entries",
        "delete_entries",
        "capture_fill_target",
        "fill_credential",
        "write_credential",
    ] {
        assert!(!mcp.tool_allowed(name).unwrap());
    }
    assert!(mcp.tool_allowed("get_mcp_config").unwrap());
    assert!(!dir.path().join("denied.txt").exists());
    assert_eq!(
        desktop.execute(field(&entry.id, entry.revision)).unwrap(),
        "FAKE_CORE_TEST_SECRET_8192"
    );
    assert_eq!(
        desktop
            .execute(Operation::Search(Search::new("".into(), None)))
            .unwrap()["entries"][0]["id"],
        entry.id
    );
    // Re-enable without replacing or reconnecting the MCP session.
    desktop
        .execute(Operation::SaveSettings(Settings::default()))
        .unwrap();
    assert!(mcp.execute(Operation::Read(vec![entry.id])).is_ok());
    assert!(mcp.tool_allowed("get_entries").unwrap());
}
#[test]
fn file_and_window_permissions_are_independent_and_checked_before_execution() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = open(dir.path(), false).unwrap();
    let mcp = open(dir.path(), true).unwrap();
    let entry = create(&desktop, "destination fixture");
    let mut policy = settings(&desktop);
    policy.mcp_permissions.fill_input = false;
    desktop
        .execute(Operation::SaveSettings(policy.clone()))
        .unwrap();
    assert!(mcp.tool_allowed("write_credential").unwrap());
    assert_eq!(
        mcp.execute(Operation::CaptureTarget(CaptureRequest::default())).unwrap_err().code,
        "PERMISSION_DENIED"
    );
    assert!(
        mcp.execute(write(&entry.id, &dir.path().join("allowed.txt")))
            .is_ok()
    );
    assert_eq!(
        mcp.execute(Operation::CaptureTarget(CaptureRequest {
            window_title: Some("Must not be focused".into()),
            field_name: Some("Password".into()),
            expected_password: Some(true),
            ..Default::default()
        })).unwrap_err().code,
        "PERMISSION_DENIED"
    );
    policy.mcp_permissions.write_file = false;
    policy.mcp_permissions.fill_input = true;
    desktop.execute(Operation::SaveSettings(policy)).unwrap();
    assert!(mcp.tool_allowed("write_credential").unwrap());
    assert_eq!(
        mcp.execute(write(&entry.id, &dir.path().join("denied.txt")))
            .unwrap_err()
            .code,
        "PERMISSION_DENIED"
    );
    let input=serde_json::from_value(json!({"credentialId":entry.id,"field":"secret","target":{"type":"input","targetId":"invalid"}})).unwrap();
    assert_eq!(
        mcp.execute(Operation::Write(input)).unwrap()["status"],
        "target_unavailable"
    );
    assert!(!dir.path().join("denied.txt").exists());
}
#[test]
fn legacy_settings_preserve_enabled_behavior_and_master_switch_revokes_config() {
    let old: Settings =
        serde_json::from_value(json!({"mcpEnabled":true,"closeToTray":false})).unwrap();
    assert!(
        old.mcp_permissions.read_metadata
            && old.mcp_permissions.create
            && old.mcp_permissions.update
            && old.mcp_permissions.delete
            && old.mcp_permissions.fill_input
            && old.mcp_permissions.write_file
    );
    let dir = tempfile::tempdir().unwrap();
    let desktop = open(dir.path(), false).unwrap();
    let mcp = open(dir.path(), true).unwrap();
    let mut settings = old;
    settings.mcp_enabled = false;
    desktop.execute(Operation::SaveSettings(settings)).unwrap();
    assert_eq!(
        mcp.execute(Operation::McpConfig).unwrap_err().code,
        "MCP_DISABLED"
    );
    assert!(!mcp.tool_allowed("get_mcp_config").unwrap());
    assert!(desktop.execute(Operation::Settings).is_ok());
}
#[test]
fn mcp_mutations_return_minimal_results_and_ignore_no_identity_payload() {
    let dir = tempfile::tempdir().unwrap();
    let mcp = open(dir.path(), true).unwrap();
    let result = mcp
        .execute(Operation::Create(vec![input("private title")]))
        .unwrap();
    assert_eq!(result[0].as_object().unwrap().len(), 3);
    assert!(result[0].get("title").is_none());
    assert!(
        serde_json::from_value::<EntryInput>(
            json!({"kind":"password","title":"spoof","secret":"fake","caller":"desktop"})
        )
        .is_err()
    );
    assert!(!mcp.tool_allowed("empty_trash").unwrap());
    assert!(!mcp.tool_allowed("save_settings").unwrap());
}

#[test]
fn updates_are_desktop_only_even_if_mcp_bypasses_tool_discovery() {
    let directory = tempfile::tempdir().unwrap();
    let mcp = open(directory.path(), true).unwrap();
    for operation in [
        Operation::UpdateStatus,
        Operation::CheckUpdate,
        Operation::DownloadUpdate,
        Operation::CancelUpdate,
        Operation::InstallUpdate,
    ] {
        assert_eq!(
            mcp.execute(operation).unwrap_err().code,
            "PERMISSION_DENIED"
        );
    }
    for name in ["check_update", "download_update", "install_update"] {
        assert!(!mcp.tool_allowed(name).unwrap());
    }
}
#[test]
fn adapters_only_hold_bound_sessions_and_enter_operation_contract() {
    for source in [include_str!("../main.rs"), include_str!("../mcp.rs")] {
        for bypass in [
            "crate::vault",
            "vault::Vault",
            "crate::credential_write",
            "crate::fill",
            "credential_write::execute",
            "crypto::",
            "rusqlite::",
            ".service.",
        ] {
            assert!(!source.contains(bypass), "adapter bypasses core: {bypass}");
        }
    }
    let mcp = include_str!("../mcp.rs");
    for bypass in [
        "core::open(",
        "Operation::ReadField",
        "Operation::EmptyTrash",
        "Operation::SaveSettings",
        ".protected_values(",
    ] {
        assert!(
            !mcp.contains(bypass),
            "MCP adapter unexpectedly uses {bypass}"
        );
    }
}

#[test]
fn configuration_lowercases_only_product_path_segments() {
    assert_eq!(configuration_path(Path::new(r"D:\Vault\Vault.exe")),std::path::PathBuf::from(r"D:\vault\vault.exe"));
    assert_eq!(configuration_path(Path::new(r"D:\PrivateData\MyVault\data")),std::path::PathBuf::from(r"D:\PrivateData\MyVault\data"));
}

#[test]
fn filling_business_code_cannot_bypass_windows_capability_service() {
    for source in [
        include_str!("filling.rs"),
        include_str!("filling/write.rs"),
        include_str!("../fill.rs"),
    ] {
        for native in [
            "Win32::", "SendInput", "ReplaceFileW", "GetForegroundWindow", "UIA_",
            r#"extern "system""#,
        ] {
            assert!(!source.contains(native), "business module contains native API: {native}");
        }
    }
}

#[test]
fn desktop_and_mcp_sessions_share_one_core_without_sharing_identity() {
    let directory = tempfile::tempdir().unwrap();
    let desktop = open(directory.path(), false).unwrap();
    let mcp = desktop.mcp_session();
    assert!(Arc::ptr_eq(&desktop.service, &mcp.service));
    assert!(Arc::ptr_eq(&desktop.execution_gate, &mcp.execution_gate));
    assert!(Arc::ptr_eq(&desktop.guard, &mcp.guard));
    assert!(mcp.mcp_session().caller == permissions::Caller::Mcp);
    assert!(desktop.caller == permissions::Caller::Desktop);
    assert_eq!(mcp.execute(Operation::Settings).unwrap_err().code, "PERMISSION_DENIED");
    assert!(desktop.execute(Operation::Settings).is_ok());
}
