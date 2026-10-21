//! Local filling service. It receives storage access internally, never from adapters.
use super::*;
use crate::fill::Targets;
#[derive(Default)]
pub(super) struct FillingService {
    targets: Targets,
}
impl FillingService {
    pub fn capture(&self) -> serde_json::Value {
        serde_json::json!(self.targets.capture())
    }
    pub fn write(
        &self,
        storage: &storage::StorageService,
        args: WriteArgs,
    ) -> Result<serde_json::Value> {
        crate::credential_write::execute(&storage.vault, &self.targets, args)
    }
    pub fn fill(
        &self,
        storage: &storage::StorageService,
        args: FillRequest,
    ) -> Result<serde_json::Value> {
        Ok(serde_json::json!(self.targets.fill(
            &args.target_id,
            || crate::credential_write::resolve_field(
                &storage.vault,
                &args.credential_id,
                &args.field,
                args.service_id.as_deref()
            )
        )?))
    }
}
