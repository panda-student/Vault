//! Transport-neutral input request/result contracts. No native operations live here.
use serde::{Deserialize, Serialize};
#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureRequest {
    pub window_title: Option<String>,
    pub process_id: Option<u32>,
    pub field_name: Option<String>,
    pub expected_password: Option<bool>,
}
impl CaptureRequest {
    pub(crate) fn validate(&self) -> crate::error::Result<()> {
        for (field, value, limit) in [
            ("windowTitle", &self.window_title, 512),
            ("fieldName", &self.field_name, 256),
        ] {
            if value.as_ref().is_some_and(|s| {
                s.trim().is_empty() || s.chars().count() > limit || s.chars().any(char::is_control)
            }) {
                return Err(crate::error::AppError::invalid().with(
                    field,
                    "Use a nonempty literal label within the length limit.",
                ));
            }
        }
        if self.process_id == Some(0)
            || ((self.window_title.is_some() || self.process_id.is_some())
                && self.field_name.is_none())
        {
            return Err(crate::error::AppError::invalid().with(
                "fieldName",
                "A window selector requires a named input field; processId must be positive.",
            ));
        }
        Ok(())
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FillStatus {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_password: Option<bool>,
}
impl FillStatus {
    pub fn status(status: &'static str) -> Self {
        Self {
            status,
            target_id: None,
            expires_in_seconds: None,
            process_id: None,
            reason: None,
            is_password: None,
        }
    }
}
