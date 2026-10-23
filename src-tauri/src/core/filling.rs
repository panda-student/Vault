//! Local filling service. It receives storage access internally, never from adapters.
use super::*;
mod write;
pub use write::WriteArgs;
#[derive(Default)]
pub(super) struct FillingService;
impl FillingService {
    pub fn capture(
        &self,
        windows: &windows::WindowsService,
        request: CaptureRequest,
    ) -> Result<serde_json::Value> {
        Ok(serde_json::json!(windows.capture_target(request)?))
    }
    pub fn write(
        &self,
        storage: &storage::StorageService,
        windows: &windows::WindowsService,
        args: WriteArgs,
    ) -> Result<serde_json::Value> {
        write::execute(&storage.vault, windows, args)
    }
    pub fn fill(
        &self,
        storage: &storage::StorageService,
        windows: &windows::WindowsService,
        args: FillRequest,
    ) -> Result<serde_json::Value> {
        Ok(serde_json::json!(windows.fill_target(
            &args.target_id,
            || write::resolve_field(
                &storage.vault,
                &args.credential_id,
                &args.field,
                args.service_id.as_deref()
            )
        )?))
    }
}
