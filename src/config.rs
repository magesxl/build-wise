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
        let file = parse_yaml_file(path)?;
        let (deepseek_api_key, mcp_mongodb_uri) = load_secrets_from_env()?;

        Ok(Config {
            server: file.server,
            deepseek: file.deepseek,
            mcp: McpConfig {
                server_command: file.mcp.server_command,
                ..file.mcp
            },
            schema: file.schema,
            deepseek_api_key,
            mcp_mongodb_uri,
        })
    }
}

#[derive(Deserialize)]
struct FileConfig {
    server: ServerConfig,
    deepseek: DeepSeekConfig,
    mcp: McpConfig,
    schema: SchemaConfig,
}

fn parse_yaml_file(path: impl AsRef<Path>) -> Result<FileConfig> {
    let content = std::fs::read_to_string(path.as_ref()).context("读取 config.yaml 失败")?;
    serde_yaml::from_str(&content).context("解析 config.yaml 失败")
}

fn load_secrets_from_env() -> Result<(String, String)> {
    let api_key = std::env::var("DEEPSEEK_API_KEY")
        .context("缺少环境变量 DEEPSEEK_API_KEY（请在 .env 中设置）")?;
    let mongo_uri = std::env::var("MDB_MCP_CONNECTION_STRING")
        .context("缺少环境变量 MDB_MCP_CONNECTION_STRING（请在 .env 中设置）")?;
    Ok((api_key, mongo_uri))
}
