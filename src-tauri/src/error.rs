use serde::Serialize;

/// One concrete validation failure, exposed to the desktop UI and MCP clients
/// so both a human and an AI caller can correct the offending field directly.
#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub field: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct AppError {
    pub code: &'static str,
    pub message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Vec<Issue>>,
}

impl AppError {
    pub const fn new(code: &'static str, message: &'static str) -> Self {
        Self {
            code,
            message,
            details: None,
        }
    }
    pub const fn storage() -> Self {
        Self::new(
            "STORAGE_ERROR",
            "无法读写密码库。请检查目录权限、磁盘空间以及数据库是否完整。",
        )
    }
    pub const fn invalid() -> Self {
        Self::new(
            "INVALID_INPUT",
            "参数不正确：请检查必填字段、长度、类别和版本号。",
        )
    }
    /// Attach a concrete field-level reason to a generic validation error.
    pub fn with(mut self, field: &str, reason: &str) -> Self {
        self.details
            .get_or_insert_with(Vec::new)
            .push(Issue {
                field: field.to_string(),
                reason: reason.to_string(),
            });
        self
    }
    pub const fn corrupt() -> Self {
        Self::new(
            "DECRYPT_FAILED",
            "凭据完整性校验或解密失败，原数据未被覆盖。",
        )
    }
    pub const fn conflict() -> Self {
        Self::new("CONFLICT", "条目已被其他窗口或 AI 修改，请刷新后重试。")
    }
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)?;
        if let Some(details) = &self.details {
            for issue in details {
                write!(f, "（{}：{}）", issue.field, issue.reason)?;
            }
        }
        Ok(())
    }
}
impl std::error::Error for AppError {}
impl From<rusqlite::Error> for AppError {
    fn from(_: rusqlite::Error) -> Self {
        Self::storage()
    }
}
pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    // 结构化错误必须同时保留笼统文案与字段级细节，GUI 与 MCP 才能各自消费。
    #[test]
    fn invalid_error_carries_field_details() {
        let error = AppError::invalid().with("services", "服务地址与 url 不一致");
        assert_eq!(error.code, "INVALID_INPUT");
        let details = error.details.as_ref().unwrap();
        assert_eq!(details.len(), 1);
        assert_eq!(details[0].field, "services");
        assert_eq!(details[0].reason, "服务地址与 url 不一致");
        assert!(error.to_string().contains("services"));
    }

    // 未附加细节的错误保持旧行为：序列化输出不出现 details 字段。
    #[test]
    fn plain_error_omits_details_in_json() {
        let value = serde_json::to_value(AppError::invalid()).unwrap();
        assert!(value.get("details").is_none());
        assert!(value.get("message").is_some());
    }

    // 多个校验失败应按顺序累积，便于一次性纠正所有字段。
    #[test]
    fn multiple_issues_accumulate_in_order() {
        let error = AppError::invalid()
            .with("title", "名称不能为空")
            .with("url", "长度超过 8192");
        let details = error.details.unwrap();
        assert_eq!(details[0].field, "title");
        assert_eq!(details[1].field, "url");
    }
}
