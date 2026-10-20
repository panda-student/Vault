use crate::{
    error::{AppError, Result},
    vault::Vault,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteArgs {
    credential_id: String,
    service_id: Option<String>,
    field: String,
    target: Destination,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Destination {
    Input {
        #[serde(rename = "targetId")]
        target_id: String,
    },
    File {
        path: PathBuf,
        #[serde(default)]
        mode: FileMode,
        template: Option<String>,
        placeholder: Option<String>,
    },
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FileMode {
    #[default]
    Create,
    Replace,
}
fn failed(code: &'static str) -> AppError {
    AppError::new(
        code,
        "Local credential write failed; no file contents are returned.",
    )
}
pub fn execute(vault: &Vault, targets: &crate::fill::Targets, args: WriteArgs) -> Result<Value> {
    let resolve = || {
        resolve_field(
            vault,
            &args.credential_id,
            &args.field,
            args.service_id.as_deref(),
        )
    };
    match args.target {
        Destination::Input { target_id } => Ok(json!(targets.fill(&target_id, resolve)?)),
        Destination::File {
            path,
            mode,
            template,
            placeholder,
        } => {
            // Absolute local paths only: no shell, script, clipboard or URL transport.
            if !path.is_absolute() || path.file_name().is_none() {
                return Err(failed("ABSOLUTE_FILE_PATH_REQUIRED"));
            }
            let marker = placeholder.as_deref().unwrap_or("{{credential}}");
            if marker.is_empty() || marker.len() > 4096 {
                return Err(AppError::invalid());
            }
            let existing = matches!(mode, FileMode::Replace);
            let source = Zeroizing::new(if existing {
                if template.is_some() {
                    return Err(AppError::invalid());
                }
                let metadata =
                    fs::symlink_metadata(&path).map_err(|_| failed("FILE_UNAVAILABLE"))?;
                if !metadata.is_file()
                    || metadata.permissions().readonly()
                    || metadata.len() > 4 * 1024 * 1024
                {
                    return Err(failed("FILE_NOT_WRITABLE"));
                }
                fs::read_to_string(&path).map_err(|_| failed("UTF8_FILE_REQUIRED"))?
            } else {
                template.unwrap_or_else(|| "{{credential}}".into())
            });
            if source.len() > 4 * 1024 * 1024 || source.matches(marker).count() != 1 {
                return Err(failed("PLACEHOLDER_MUST_OCCUR_ONCE"));
            }
            let secret = Zeroizing::new(resolve()?);
            let rendered = Zeroizing::new(source.replacen(marker, &secret, 1));
            if existing {
                // Stage alongside destination, then atomically replace; no partial
                // original file on write failure. Remove the staging file on errors.
                let temporary =
                    path.with_file_name(format!(".vault-write-{}.tmp", uuid::Uuid::new_v4()));
                let result = (|| {
                    let mut file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&temporary)
                        .map_err(|_| failed("FILE_NOT_WRITABLE"))?;
                    file.write_all(rendered.as_bytes())
                        .and_then(|_| file.sync_all())
                        .map_err(|_| failed("FILE_WRITE_FAILED"))?;
                    drop(file);
                    // Detect changes while resolving the credential before replacing.
                    let current = Zeroizing::new(
                        fs::read_to_string(&path).map_err(|_| failed("FILE_UNAVAILABLE"))?,
                    );
                    if *current != *source {
                        return Err(failed("FILE_CHANGED"));
                    }
                    // ReplaceFile preserves the existing destination ACL and metadata.
                    use std::os::windows::ffi::OsStrExt;
                    use windows::{
                        Win32::Storage::FileSystem::{REPLACE_FILE_FLAGS, ReplaceFileW},
                        core::PCWSTR,
                    };
                    let destination: Vec<u16> =
                        path.as_os_str().encode_wide().chain(Some(0)).collect();
                    let replacement: Vec<u16> =
                        temporary.as_os_str().encode_wide().chain(Some(0)).collect();
                    unsafe {
                        ReplaceFileW(
                            PCWSTR(destination.as_ptr()),
                            PCWSTR(replacement.as_ptr()),
                            PCWSTR::null(),
                            REPLACE_FILE_FLAGS(0),
                            None,
                            None,
                        )
                    }
                    .map_err(|_| failed("FILE_REPLACE_FAILED"))?;
                    Ok(())
                })();
                if result.is_err() {
                    let _ = fs::remove_file(&temporary);
                }
                result?;
            } else {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|_| failed("FILE_EXISTS_OR_NOT_WRITABLE"))?;
                if file
                    .write_all(rendered.as_bytes())
                    .and_then(|_| file.sync_all())
                    .is_err()
                {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(failed("FILE_WRITE_FAILED"));
                }
            }
            Ok(json!({"status":"file_written"}))
        }
    }
}

/// Shared local resolution: no plaintext from this function is an MCP response.
pub fn resolve_field(
    vault: &Vault,
    id: &str,
    field: &str,
    service_id: Option<&str>,
) -> Result<String> {
    let entry = vault.get(id)?;
    Ok(match field {
        "account" => entry.summary.account,
        "secret" => entry.secret,
        "url" => entry.summary.url,
        "service_url" => entry.summary.service(service_id)?.url.clone(),
        "host" | "port" | "database_name" | "database_type" => {
            let db = entry.summary.database.ok_or_else(AppError::invalid)?;
            match field {
                "host" => db.host,
                "port" => db.port,
                "database_name" => db.database_name,
                _ => db.database_type,
            }
        }
        _ => return Err(AppError::invalid()),
    })
}
