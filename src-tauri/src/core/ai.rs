//! AI-safe projection: redact before filtering to prevent a search oracle.
use super::*;
use crate::ai_output::SharedGuard;

use serde_json::{Value, json};
pub(super) fn metadata(
    core: &storage::StorageService,
    guard: &SharedGuard,
    id: &str,
) -> Result<Value> {
    let summary = core.summary(id)?;
    crate::ai_output::scrub(
        guard,
        json!({"id":summary.id,"kind":summary.kind,"title":summary.title,"account":summary.account,"url":summary.url,"services":summary.services,"group":summary.group,"tags":summary.tags,"database":summary.database,"revision":summary.revision,"pinned":summary.pinned,"createdAt":summary.created_at,"updatedAt":summary.updated_at}),
    )
}
pub(super) fn search(
    core: &storage::StorageService,
    guard: &SharedGuard,
    args: Search,
    settings: &Settings,
) -> Result<Value> {
    let query = args.query.to_lowercase();
    if query.len() > 4096 {
        return Err(AppError::invalid().with("query", "搜索词长度不能超过 4096。"));
    }
    let mut entries = Vec::new();
    for summary in core.list("", args.kind)? {
        if !super::permissions::permitted(
            settings,
            Some(&summary.id),
            &summary.tags,
            super::permissions::Access::Read,
        ) {
            continue;
        }
        let item = metadata(core, guard, &summary.id)?;
        if args.group.as_ref().is_some_and(|group| {
            !item["tags"]
                .as_array()
                .is_some_and(|tags| tags.iter().any(|t| t.as_str() == Some(group.as_str())))
        }) || args.tag.as_ref().is_some_and(|tag| {
            !item["tags"]
                .as_array()
                .is_some_and(|tags| tags.iter().any(|t| t.as_str() == Some(tag.as_str())))
        }) {
            continue;
        }
        // Searching redacted fields would become an oracle for secrets
        // accidentally placed in names, accounts or URLs.
        if query.is_empty()
            || ["title", "account", "url", "group"].iter().any(|field| {
                item[*field]
                    .as_str()
                    .is_some_and(|v| v.to_lowercase().contains(&query))
            })
            || item["tags"].as_array().is_some_and(|tags| {
                tags.iter().any(|t| {
                    t.as_str()
                        .is_some_and(|s| s.to_lowercase().contains(&query))
                })
            })
            || item["database"].as_object().is_some_and(|db| {
                db.values().any(|v| {
                    v.as_str()
                        .is_some_and(|s| s.to_lowercase().contains(&query))
                })
            })
            || item["services"].as_array().is_some_and(|services| {
                services.iter().any(|service| {
                    ["protocol", "url"].iter().any(|field| {
                        service[*field]
                            .as_str()
                            .is_some_and(|v| v.to_lowercase().contains(&query))
                    })
                })
            })
        {
            entries.push(item);
        }
    }
    let has_more = entries.len() > 100;
    entries.truncate(100);
    Ok(json!({"entries":entries,"hasMore":has_more}))
}
