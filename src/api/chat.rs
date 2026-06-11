use crate::ai::deepseek;
use crate::config::Config;
use crate::mcp::client::McpClient;
use anyhow::Result;
use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionMessageToolCall, ChatCompletionRequestAssistantMessageArgs,
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestToolMessageArgs, ChatCompletionRequestUserMessageArgs,
        ChatCompletionToolType, CreateChatCompletionRequestArgs, FinishReason,
    },
    Client,
};
use axum::{
    extract::State,
    response::{
        sse::{Event, KeepAlive},
        Sse,
    },
    Json,
};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

#[derive(Debug, Clone)]
pub enum SseEvent {
    Content(String),
    Done,
    Error(String),
}

impl SseEvent {
    pub fn to_json(&self) -> String {
        match self {
            SseEvent::Content(text) => {
                serde_json::json!({"type":"content","text":text}).to_string()
            }
            SseEvent::Done => serde_json::json!({"type":"done"}).to_string(),
            SseEvent::Error(msg) => serde_json::json!({"type":"error","text":msg}).to_string(),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct ChatRequest {
    pub model_ids: Vec<String>,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// 应用共享状态
pub struct AppState {
    pub config: Arc<Config>,
    pub mcp_client: Arc<McpClient>,
    pub cancel_token: tokio::sync::Mutex<Option<tokio_util::sync::CancellationToken>>,
}

pub async fn run_analysis(
    config: Arc<Config>,
    mcp_client: Arc<McpClient>,
    request: ChatRequest,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<mpsc::Receiver<SseEvent>> {
    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(async move {
        if let Err(e) = run_analysis_inner(config, mcp_client, request, tx.clone(), cancel).await {
            tracing::error!("分析失败: {:?}", e);
            let _ = tx.send(SseEvent::Error(format!("系统错误: {}", e))).await;
        }
    });
    Ok(rx)
}

async fn run_analysis_inner(
    config: Arc<Config>,
    mcp_client: Arc<McpClient>,
    request: ChatRequest,
    tx: mpsc::Sender<SseEvent>,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<()> {
    let openai_cfg = OpenAIConfig::default()
        .with_api_key(&config.deepseek_api_key)
        .with_api_base(&config.deepseek.base_url);
    let client = Client::with_config(openai_cfg);

    // 构建消息列表
    let mut messages: Vec<ChatCompletionRequestMessage> = Vec::new();

    // System prompt
    let sys_prompt = deepseek::build_system_prompt(&config);
    messages.push(
        ChatCompletionRequestSystemMessageArgs::default()
            .content(sys_prompt)
            .build()?
            .into(),
    );

    // 模型 ID 上下文
    messages.push(
        ChatCompletionRequestSystemMessageArgs::default()
            .content(format!(
                "用户指定的模型 ID: [{}]。请针对这些模型进行分析。",
                request.model_ids.join(", ")
            ))
            .build()?
            .into(),
    );

    // 对话历史 + 当前问题
    for msg in &request.messages {
        match msg.role.as_str() {
            "user" => {
                messages.push(
                    ChatCompletionRequestUserMessageArgs::default()
                        .content(msg.content.clone())
                        .build()?
                        .into(),
                );
            }
            "assistant" => {
                messages.push(
                    ChatCompletionRequestAssistantMessageArgs::default()
                        .content(msg.content.clone())
                        .build()?
                        .into(),
                );
            }
            _ => {}
        }
    }

    // 如果没有用户消息，补一条默认
    if request.messages.is_empty()
        || request.messages.last().map(|m| m.role.as_str()) != Some("user")
    {
        messages.push(
            ChatCompletionRequestUserMessageArgs::default()
                .content(format!(
                    "请分析模型 {} 的建造信息。",
                    request.model_ids.join(", ")
                ))
                .build()?
                .into(),
        );
    }

    // 获取 MCP Server 的 tools 并合并 describe_model_schema
    let mcp_tools = mcp_client.tools().await?;
    tracing::info!(
        "MCP tools 可用: {:?}",
        mcp_tools.iter().map(|t| &t.name).collect::<Vec<_>>()
    );

    let mut tools: Vec<_> = deepseek::mcp_tools_to_openai(&mcp_tools, &config);
    tools.push(deepseek::describe_model_schema_tool(&config));
    tracing::info!("已注册 {} 个 tools 给 DeepSeek", tools.len());

    // 对话循环：最多 20 轮 tool calling
    const MAX_ROUNDS: usize = 30;
    tracing::info!(
        "开始 AI 对话，模型: {}，消息数: {}",
        config.deepseek.model,
        messages.len()
    );
    for round in 0..MAX_ROUNDS {
        // 检查点 1：每轮循环开始
        if cancel.is_cancelled() {
            tracing::info!("收到取消信号（第 {} 轮开始），终止分析", round + 1);
            return Ok(());
        }
        tracing::info!("第 {}/{} 轮对话", round + 1, MAX_ROUNDS);
        let req = CreateChatCompletionRequestArgs::default()
            .model(&config.deepseek.model)
            .messages(messages.clone())
            .tools(tools.clone())
            .max_tokens(config.deepseek.max_tokens)
            .build()?;

        let mut stream = client.chat().create_stream(req).await?;

        // 收集本轮 tool calls（可能来自多个 chunk）
        let mut tool_call_chunks: Vec<ChatCompletionMessageToolCall> = Vec::new();
        let mut finish_reason: Option<FinishReason> = None;

        while let Some(chunk_result) = stream.next().await {
            // 检查点 2：每个 SSE chunk 后
            if cancel.is_cancelled() {
                tracing::info!("收到取消信号，中断 DeepSeek 流式响应");
                drop(stream); // 断开 HTTP 连接
                return Ok(());
            }
            let chunk = match chunk_result {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx
                        .send(SseEvent::Error(format!("AI 服务错误: {}", e)))
                        .await;
                    return Ok(());
                }
            };

            for choice in &chunk.choices {
                // 文本 delta → 直接推 SSE
                if let Some(ref content) = choice.delta.content {
                    if !content.is_empty() {
                        if tx.send(SseEvent::Content(content.clone())).await.is_err() {
                            return Ok(()); // 客户端断开
                        }
                    }
                }

                // Tool call delta → 按 index 合并
                if let Some(ref tc_deltas) = choice.delta.tool_calls {
                    for tc_delta in tc_deltas {
                        let idx = tc_delta.index as usize;
                        while tool_call_chunks.len() <= idx {
                            tool_call_chunks.push(ChatCompletionMessageToolCall {
                                id: String::new(),
                                r#type: ChatCompletionToolType::Function,
                                function: async_openai::types::FunctionCall {
                                    name: String::new(),
                                    arguments: String::new(),
                                },
                            });
                        }
                        let target = &mut tool_call_chunks[idx];
                        if let Some(ref id) = tc_delta.id {
                            target.id = id.clone();
                        }
                        if let Some(ref func) = tc_delta.function {
                            if let Some(ref name) = func.name {
                                target.function.name = name.clone();
                            }
                            if let Some(ref args) = func.arguments {
                                target.function.arguments.push_str(args);
                            }
                        }
                    }
                }

                // 收集 finish_reason
                if choice.finish_reason.is_some() {
                    finish_reason = choice.finish_reason.clone();
                }
            }
        }

        // 处理本轮结束状态
        tracing::debug!(
            "finish_reason: {:?}, tool_calls: {}",
            finish_reason,
            tool_call_chunks.len()
        );
        match finish_reason {
            Some(FinishReason::ToolCalls) => {
                tracing::info!("AI 请求 {} 个 tool calls", tool_call_chunks.len());
                if tool_call_chunks.is_empty() {
                    let _ = tx
                        .send(SseEvent::Error("AI 请求执行工具但未指定工具".into()))
                        .await;
                    return Ok(());
                }

                // 添加 assistant message（含 tool_calls）
                messages.push(
                    ChatCompletionRequestAssistantMessageArgs::default()
                        .tool_calls(tool_call_chunks.clone())
                        .build()?
                        .into(),
                );

                // 执行每个 tool call
                for tc in &tool_call_chunks {
                    tracing::info!("执行 tool: {} (id: {})", tc.function.name, tc.id);
                    let result = match tc.function.name.as_str() {
                        "describe_model_schema" => deepseek::handle_describe_schema(&config),
                        other => {
                            let args: Value =
                                serde_json::from_str(&tc.function.arguments).unwrap_or(Value::Null);
                            match mcp_client.call_tool(other, args).await {
                                Ok(text) => deepseek::format_tool_result(other, &text),
                                Err(e) => format!("工具调用失败: {}", e),
                            }
                        }
                    };

                    messages.push(
                        ChatCompletionRequestToolMessageArgs::default()
                            .tool_call_id(tc.id.clone())
                            .content(result)
                            .build()?
                            .into(),
                    );

                    // 检查点 3：每个 MCP tool call 返回后
                    if cancel.is_cancelled() {
                        tracing::info!("收到取消信号（tool call 后），终止分析");
                        return Ok(());
                    }
                }

                // 继续循环，让 AI 看到 tool 结果后重新回答
                continue;
            }
            _ => {
                tracing::info!("AI 回答完成");
                let _ = tx.send(SseEvent::Done).await;
                return Ok(());
            }
        }
    }

    tracing::warn!("达到最大对话轮次");
    let _ = tx
        .send(SseEvent::Error("分析轮次过多，请简化提问".into()))
        .await;
    Ok(())
}

// ── HTTP handlers ──────────────────────────────────────────────

pub async fn chat_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ChatRequest>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    tracing::info!("收到请求，模型 IDs: {:?}", request.model_ids);
    let config = state.config.clone();
    let mcp = state.mcp_client.clone();

    // 创建新的取消令牌，替换旧令牌（旧的会被 cancel 掉）
    let cancel = tokio_util::sync::CancellationToken::new();
    {
        let mut guard = state.cancel_token.lock().await;
        if let Some(old) = guard.take() {
            old.cancel();
        }
        *guard = Some(cancel.clone());
    }

    let rx = run_analysis(config, mcp, request, cancel)
        .await
        .unwrap_or_else(|e| {
            let (tx, rx) = mpsc::channel(1);
            let _ = tx.try_send(SseEvent::Error(format!("请求处理失败: {}", e)));
            rx
        });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx)
        .map(|event| Ok(Event::default().data(event.to_json())));

    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub async fn cancel_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let mut guard = state.cancel_token.lock().await;
    if let Some(token) = guard.take() {
        token.cancel();
        tracing::info!("已取消当前分析任务");
        Json(serde_json::json!({"ok": true, "message": "已取消"}))
    } else {
        Json(serde_json::json!({"ok": true, "message": "无进行中的任务"}))
    }
}
