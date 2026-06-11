use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};

/// MCP JSON-RPC 请求
#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    params: Value,
}

/// MCP JSON-RPC 响应
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<u64>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    #[allow(dead_code)]
    code: i64,
    message: String,
}

type PendingRequests = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>>;

/// MCP stdio 客户端
pub struct McpClient {
    next_id: Mutex<u64>,
    pending: PendingRequests,
    stdin: Mutex<ChildStdin>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

impl McpClient {
    /// 启动 mongodb-mcp-server 子进程，完成初始化握手
    pub async fn connect(
        command: &str,
        args: &[String],
        mongodb_uri: &str,
    ) -> Result<Arc<Self>> {
        let mut child = Command::new(command)
            .args(args)
            .env("MDB_MCP_CONNECTION_STRING", mongodb_uri)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .context("启动 MCP Server 进程失败")?;
        tracing::info!("MCP 子进程 PID: {}", child.id().unwrap_or(0));

        let stdin = child.stdin.take().context("获取 stdin 失败")?;
        let stdout = child.stdout.take().context("获取 stdout 失败")?;

        let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));

        // 启动 stdout 读取任务
        let pending_clone = pending.clone();
        tokio::spawn(async move {
            read_stdout(stdout, pending_clone).await;
        });

        // 监控子进程退出
        tokio::spawn(async move {
            let status = child.wait().await;
            tracing::error!("MCP Server 进程退出: {:?}", status);
        });

        let client = Arc::new(Self {
            next_id: Mutex::new(1),
            pending,
            stdin: Mutex::new(stdin),
        });

        // MCP 协议握手：initialize → initialized
        let init_result = client
            .send_request(
                "initialize",
                serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "build-wise",
                        "version": "0.1.0"
                    }
                }),
            )
            .await?;
        tracing::info!("MCP initialize 完成: {:?}", init_result);

        // 发送 initialized 通知（不需要响应）
        client
            .send_notification("notifications/initialized", serde_json::json!({}))
            .await?;

        // 获取 tools
        let tools = client.list_tools().await?;
        tracing::info!("MCP Server 已连接，获取到 {} 个 tools", tools.len());

        Ok(client)
    }

    /// 获取可用的 tool 列表（实时查 MCP Server）
    pub async fn tools(&self) -> Result<Vec<McpTool>> {
        self.list_tools().await
    }

    /// 获取 tools/list
    async fn list_tools(&self) -> Result<Vec<McpTool>> {
        let result = self
            .send_request("tools/list", serde_json::json!({}))
            .await?;
        let tools: Vec<McpTool> = serde_json::from_value(
            result
                .get("tools")
                .context("tools/list 响应缺少 tools 字段")?
                .clone(),
        )
        .context("解析 tools 失败")?;
        Ok(tools)
    }

    /// 调用 MCP tool
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<String> {
        let result = self
            .send_request(
                "tools/call",
                serde_json::json!({
                    "name": name,
                    "arguments": arguments,
                }),
            )
            .await?;

        // 从 MCP 响应中提取文本内容
        let content = result
            .get("content")
            .context("tools/call 响应缺少 content")?;
        let text = extract_text_from_content(content)?;
        Ok(text)
    }

    /// 发送 JSON-RPC 通知（不需要响应，无 id）
    async fn send_notification(&self, method: &str, params: Value) -> Result<()> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let json = serde_json::to_string(&request)? + "\n";
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(json.as_bytes())
            .await
            .context("写入 stdin 失败")?;
        stdin.flush().await.context("flush stdin 失败")?;
        Ok(())
    }

    /// 发送 JSON-RPC 请求并等待响应
    async fn send_request(&self, method: &str, params: Value) -> Result<Value> {
        let id = {
            let mut next = self.next_id.lock().await;
            let id = *next;
            *next += 1;
            id
        };

        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        };

        let (tx, rx) = oneshot::channel();

        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        // 发送请求
        let json = serde_json::to_string(&request)? + "\n";
        {
            let mut stdin = self.stdin.lock().await;
            stdin
                .write_all(json.as_bytes())
                .await
                .context("写入 stdin 失败")?;
            stdin.flush().await.context("flush stdin 失败")?;
        }

        // 等待响应（30 秒超时）
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .context("MCP 请求超时（30s），请检查 MongoDB 连接和 MCP Server 是否正常")?
            .context("MCP 响应通道关闭（可能进程已退出）")??;

        if let Some(error) = result.get("error") {
            let msg = error.get("message").and_then(|m| m.as_str()).unwrap_or("未知错误");
            anyhow::bail!("MCP Server 返回错误: {}", msg);
        }

        Ok(result)
    }
}

/// 从 MCP content 中提取文本
fn extract_text_from_content(content: &Value) -> Result<String> {
    if let Some(arr) = content.as_array() {
        let texts: Vec<String> = arr
            .iter()
            .filter_map(|item| item.get("text").and_then(|t| t.as_str()).map(String::from))
            .collect();
        if texts.is_empty() {
            // 不是文本内容，返回 JSON 字符串
            Ok(serde_json::to_string(content)?)
        } else {
            Ok(texts.join("\n"))
        }
    } else if let Some(s) = content.as_str() {
        Ok(s.to_string())
    } else {
        Ok(serde_json::to_string(content)?)
    }
}

/// 后台读取子进程 stdout，匹配响应到对应的 oneshot
async fn read_stdout(
    stdout: tokio::process::ChildStdout,
    pending: PendingRequests,
) {
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        match serde_json::from_str::<JsonRpcResponse>(&line) {
            Ok(resp) => {
                if let Some(id) = resp.id {
                    let mut pending_map = pending.lock().await;
                    if let Some(tx) = pending_map.remove(&id) {
                        let value = if let Some(ref error) = resp.error {
                            Err(anyhow::anyhow!("{}", error.message))
                        } else {
                            Ok(resp.result.unwrap_or(Value::Null))
                        };
                        let _ = tx.send(value);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("无法解析 MCP 响应行: {} - {}", line, e);
            }
        }
    }
}
