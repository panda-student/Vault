use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Password,
    Api,
    Database,
    Generic,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiProtocol {
    Openai,
    Anthropic,
    Gemini,
    Custom,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiService {
    #[serde(default)]
    pub id: String,
    pub protocol: ApiProtocol,
    pub url: String,
}
impl EntrySummary {
    pub fn service(&self, id: Option<&str>) -> Result<&ApiService> {
        if self.kind != Kind::Api {
            return Err(AppError::invalid());
        }
        if let Some(id) = id {
            return self
                .services
                .iter()
                .find(|s| s.id == id)
                .ok_or(AppError::new(
                    "SERVICE_NOT_FOUND",
                    "服务地址不存在，请刷新条目。",
                ));
        }
        if self.services.len() > 1 {
            return Err(AppError::new(
                "SERVICE_REQUIRED",
                "此凭据包含多个服务，请指定 serviceId。",
            ));
        }
        self.services.first().ok_or(AppError::new(
            "SERVICE_REQUIRED",
            "请先为此凭据添加服务地址。",
        ))
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntryInput {
    pub kind: Kind,
    pub title: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub database: Option<DatabaseFields>,

    #[serde(default)]
    pub account: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub documentation_url: String,
    #[serde(default)]
    pub services: Vec<ApiService>,
    pub secret: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntryPatch {
    pub id: String,
    pub expected_revision: i64,
    pub title: Option<String>,
    pub tags: Option<Vec<String>>,
    pub database: Option<DatabaseFields>,
    pub account: Option<String>,
    pub url: Option<String>,
    pub documentation_url: Option<String>,
    pub services: Option<Vec<ApiService>>,
    pub secret: Option<String>,
    pub notes: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntryVersion {
    pub id: String,
    pub expected_revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntrySummary {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub database: Option<DatabaseFields>,

    pub account: String,
    pub url: String,
    #[serde(default)]
    pub documentation_url: String,
    pub services: Vec<ApiService>,
    pub notes: String,
    pub pinned: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub revision: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryDetail {
    #[serde(flatten)]
    pub summary: EntrySummary,
    pub secret: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub mcp_enabled: bool,
    pub close_to_tray: bool,
    pub mcp_permissions: McpPermissions,
    pub mcp_access: bool,
    pub mcp_tag_rules: std::collections::BTreeMap<String, PermissionRule>,
    pub mcp_entry_rules: std::collections::BTreeMap<String, PermissionRule>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PermissionRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_metadata: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_input: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_file: Option<bool>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            mcp_enabled: true,
            close_to_tray: false,
            mcp_permissions: McpPermissions::default(),
            mcp_access: true,
            mcp_tag_rules: Default::default(),
            mcp_entry_rules: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpPermissions {
    pub read_metadata: bool,
    pub create: bool,
    pub update: bool,
    pub delete: bool,
    pub fill_input: bool,
    pub write_file: bool,
}
impl Default for McpPermissions {
    fn default() -> Self {
        Self {
            read_metadata: true,
            create: true,
            update: true,
            delete: true,
            fill_input: true,
            write_file: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatabaseFields {
    #[serde(default)]
    pub database_type: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: String,
    #[serde(default)]
    pub database_name: String,
}
