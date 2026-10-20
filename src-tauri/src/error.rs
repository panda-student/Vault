use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct AppError {
    pub code: &'static str,
    pub message: &'static str,
}

impl AppError {
    pub const fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
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
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for AppError {}
impl From<rusqlite::Error> for AppError {
    fn from(_: rusqlite::Error) -> Self {
        Self::storage()
    }
}
pub type Result<T> = std::result::Result<T, AppError>;
