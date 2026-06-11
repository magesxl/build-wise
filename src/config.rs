use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub deepseek: DeepSeekConfig,
    pub mcp: McpConfig,
    pub schema: SchemaConfig,

    // 敏感信息 —— 只从环境变量 (.env) 来，不出现在 config.yaml
    pub deepseek_api_key: String,
    pub mcp_mongodb_uri: String,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct ServerConfig {
    pub port: u16,
    pub log_level: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeepSeekConfig {
    pub model: String,
    pub base_url: String,
    pub max_tokens: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct McpConfig {
    pub server_command: String,
    pub server_args: Vec<String>,
    pub database: String,
    pub collection: String,
}

/// 多集合 Schema 配置
#[derive(Debug, Clone, Deserialize)]
pub struct SchemaConfig {
    pub collections: HashMap<String, CollectionConfig>,
}

/// 单个集合的 Schema
#[derive(Debug, Clone, Deserialize)]
pub struct CollectionConfig {
    pub description: String,
    #[serde(default)]
    pub fields: HashMap<String, FieldMapping>,
    #[serde(default)]
    pub property_groups: HashMap<String, PropertyGroupMapping>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FieldMapping {
    pub zh: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PropertyGroupMapping {
    pub zh: String,
    #[serde(default)]
    pub description: Option<String>,
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let content =
            std::fs::read_to_string(path.as_ref()).context("读取 config.yaml 失败")?;

        #[derive(Deserialize)]
        struct FileConfig {
            server: ServerConfig,
            deepseek: DeepSeekConfig,
            mcp: McpConfig,
            schema: SchemaConfig,
        }

        let file: FileConfig = serde_yaml::from_str(&content).context("解析 config.yaml 失败")?;

        // 敏感信息只从环境变量读取，config.yaml 里不存
        let deepseek_api_key = std::env::var("DEEPSEEK_API_KEY")
            .context("缺少环境变量 DEEPSEEK_API_KEY（请在 .env 中设置）")?;
        let mcp_mongodb_uri = std::env::var("MDB_MCP_CONNECTION_STRING")
            .context("缺少环境变量 MDB_MCP_CONNECTION_STRING（请在 .env 中设置）")?;

        // 根据当前 OS 自动选择正确的 npx 命令
        let npx = if cfg!(windows) { "npx.cmd" } else { "npx" };
        let server_command = if file.mcp.server_command.starts_with("npx") {
            npx.to_string()
        } else {
            file.mcp.server_command
        };

        Ok(Config {
            server: file.server,
            deepseek: file.deepseek,
            mcp: McpConfig {
                server_command,
                ..file.mcp
            },
            schema: file.schema,
            deepseek_api_key,
            mcp_mongodb_uri,
        })
    }

    /// 生成所有集合的 Schema 描述文本
    pub fn schema_description(&self) -> String {
        let mut desc = String::new();

        for (name, col) in &self.schema.collections {
            desc.push_str(&format!(
                "=== 集合: {} ===\n说明: {}\n",
                name, col.description
            ));

            if !col.fields.is_empty() {
                desc.push_str("字段说明:\n");
                for (field, mapping) in &col.fields {
                    desc.push_str(&format!("  - {}: {}", field, mapping.zh));
                    if let Some(ref d) = mapping.description {
                        desc.push_str(&format!("（{}）", d));
                    }
                    desc.push('\n');
                }
            }

            if !col.property_groups.is_empty() {
                desc.push_str("propertySet 的 paramGroupId 分组:\n");
                for (group, mapping) in &col.property_groups {
                    desc.push_str(&format!("  - {}: {}", group, mapping.zh));
                    if let Some(ref d) = mapping.description {
                        desc.push_str(&format!("（{}）", d));
                    }
                    desc.push('\n');
                }
            }

            desc.push('\n');
        }

        desc.trim_end().to_string()
    }

    /// 列出所有集合名
    pub fn collection_names(&self) -> Vec<&str> {
        self.schema.collections.keys().map(|s| s.as_str()).collect()
    }
}
