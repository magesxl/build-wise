use crate::config::Config;
use crate::mcp::client::{McpClient, McpTool};
use anyhow::Result;
use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionMessageToolCall, ChatCompletionRequestAssistantMessageArgs,
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestToolMessageArgs, ChatCompletionRequestUserMessageArgs,
        ChatCompletionTool, ChatCompletionToolType, CreateChatCompletionRequestArgs, FinishReason,
        FunctionObject,
    },
    Client,
};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;

// 从 api/chat 导入 SSE 事件类型（数据载体，不引入行为依赖）
use crate::api::chat::SseEvent;

// ── 公共接口 ──────────────────────────────────────────────────

/// AI 对话驱动器。
///
/// 封装完整的 tool-calling 对话流程：prompt 构建 → MCP tool 注册 →
/// DeepSeek 流式交互 → chunk 合并 → tool 分发 → 结果回流。
///
/// Interface 极小（一个构造器 + 一个 `run()`），implementation 承载全部 AI 行为。
pub struct ConversationDriver {
    config: Arc<Config>,
    mcp_client: Arc<McpClient>,
    model_ids: Option<Vec<String>>,
    history: Vec<(String, String)>, // (role, content)
    tx: mpsc::Sender<SseEvent>,
    cancel: CancellationToken,
}

impl ConversationDriver {
    pub fn new(
        config: Arc<Config>,
        mcp_client: Arc<McpClient>,
        model_ids: Option<Vec<String>>,
        history: Vec<(String, String)>,
        tx: mpsc::Sender<SseEvent>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            config,
            mcp_client,
            model_ids,
            history,
            tx,
            cancel,
        }
    }

    /// 执行完整的 AI 对话流程。发送内容 chunk 到 tx，正常结束时不发 Done
    /// （由调用方在 `run()` 返回 Ok 后统一发送）。
    pub async fn run(self) -> Result<()> {
        let openai_cfg = OpenAIConfig::default()
            .with_api_key(&self.config.deepseek_api_key)
            .with_api_base(&self.config.deepseek.base_url);
        let client = Client::with_config(openai_cfg);

        let mut messages = self.build_messages()?;

        // 获取 MCP tools 并注册 describe_model_schema
        let mcp_tools = self.mcp_client.tools().await?;
        tracing::info!(
            "MCP tools 可用: {:?}",
            mcp_tools.iter().map(|t| &t.name).collect::<Vec<_>>()
        );
        let mut tools = mcp_tools_to_openai(&mcp_tools, &self.config);
        tools.push(describe_model_schema_tool());
        tracing::info!("已注册 {} 个 tools 给 DeepSeek", tools.len());

        // 对话循环
        const MAX_ROUNDS: usize = 30;
        tracing::info!(
            "开始 AI 对话，模型: {}，消息数: {}",
            self.config.deepseek.model,
            messages.len()
        );

        for round in 0..MAX_ROUNDS {
            if self.cancel.is_cancelled() {
                tracing::info!("收到取消信号（第 {} 轮开始），终止分析", round + 1);
                return Ok(());
            }
            tracing::info!("第 {}/{} 轮对话", round + 1, MAX_ROUNDS);

            let req = CreateChatCompletionRequestArgs::default()
                .model(&self.config.deepseek.model)
                .messages(messages.clone())
                .tools(tools.clone())
                .max_tokens(self.config.deepseek.max_tokens)
                .build()?;

            let mut stream = client.chat().create_stream(req).await?;

            let (tool_call_chunks, finish_reason) = self.stream_and_merge(&mut stream).await?;

            tracing::debug!(
                "finish_reason: {:?}, tool_calls: {}",
                finish_reason,
                tool_call_chunks.len()
            );

            match finish_reason {
                Some(FinishReason::ToolCalls) => {
                    if tool_call_chunks.is_empty() {
                        let _ = self
                            .tx
                            .send(SseEvent::Error("AI 请求执行工具但未指定工具".into()))
                            .await;
                        return Ok(());
                    }
                    self.handle_tool_calls(&mut messages, &tool_call_chunks, &client)
                        .await?;
                    continue; // 继续循环，让 AI 看到 tool 结果
                }
                _ => {
                    tracing::info!("AI 回答完成");
                    return Ok(());
                }
            }
        }

        tracing::warn!("达到最大对话轮次");
        let _ = self
            .tx
            .send(SseEvent::Error("分析轮次过多，请简化提问".into()))
            .await;
        Ok(())
    }
}

// ── 消息构建 ──────────────────────────────────────────────────

impl ConversationDriver {
    fn build_messages(&self) -> Result<Vec<ChatCompletionRequestMessage>> {
        let mut messages: Vec<ChatCompletionRequestMessage> = Vec::new();

        // System prompt
        messages.push(
            ChatCompletionRequestSystemMessageArgs::default()
                .content(build_system_prompt(&self.config))
                .build()?
                .into(),
        );

        // 模型 ID 上下文（可��）
        if let Some(ref ids) = self.model_ids {
            if !ids.is_empty() {
                messages.push(
                    ChatCompletionRequestSystemMessageArgs::default()
                        .content(format!(
                            "用户指定的模型 ID: [{}]。请针对这些模型进行分析。",
                            ids.join(", ")
                        ))
                        .build()?
                        .into(),
                );
            }
        }

        // 对话历史 + 当前问题
        for (role, content) in &self.history {
            match role.as_str() {
                "user" => {
                    messages.push(
                        ChatCompletionRequestUserMessageArgs::default()
                            .content(content.clone())
                            .build()?
                            .into(),
                    );
                }
                "assistant" => {
                    messages.push(
                        ChatCompletionRequestAssistantMessageArgs::default()
                            .content(content.clone())
                            .build()?
                            .into(),
                    );
                }
                _ => {}
            }
        }

        // 如果没有用户消息，补一条默认
        if self.history.is_empty() || self.history.last().map(|(r, _)| r.as_str()) != Some("user") {
            let fallback = match self.model_ids {
                Some(ref ids) if !ids.is_empty() => format!("请分析模型 {} 的建造信息。", ids.join(", ")),
                _ => "请分析建造信息。".to_string(),
            };
            messages.push(
                ChatCompletionRequestUserMessageArgs::default()
                    .content(fallback)
                    .build()?
                    .into(),
            );
        }

        Ok(messages)
    }
}

// ── 流式处理 ──────────────────────────────────────────────────

impl ConversationDriver {
    /// 消费 DeepSeek 流式响应，推送文本 chunk 到 tx，累积 tool call delta。
    /// 返回合并后的 tool call 列表和 finish_reason。
    async fn stream_and_merge(
        &self,
        stream: &mut (impl StreamExt<
            Item = Result<
                async_openai::types::CreateChatCompletionStreamResponse,
                async_openai::error::OpenAIError,
            >,
        > + Unpin),
    ) -> Result<(Vec<ChatCompletionMessageToolCall>, Option<FinishReason>)> {
        let mut tool_call_chunks: Vec<ChatCompletionMessageToolCall> = Vec::new();
        let mut finish_reason: Option<FinishReason> = None;

        while let Some(chunk_result) = stream.next().await {
            if self.cancel.is_cancelled() {
                tracing::info!("收到取消信号，中断 DeepSeek 流式响应");
                return Ok((tool_call_chunks, finish_reason));
            }

            let chunk = match chunk_result {
                Ok(c) => c,
                Err(e) => {
                    let _ = self
                        .tx
                        .send(SseEvent::Error(format!("AI 服务错误: {}", e)))
                        .await;
                    return Ok((tool_call_chunks, finish_reason));
                }
            };

            for choice in &chunk.choices {
                // 文本 delta → 直接推 SSE
                if let Some(ref content) = choice.delta.content {
                    if !content.is_empty()
                        && self
                            .tx
                            .send(SseEvent::Content(content.clone()))
                            .await
                            .is_err()
                    {
                        return Ok((tool_call_chunks, finish_reason)); // 客户端断开
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

                if choice.finish_reason.is_some() {
                    finish_reason = choice.finish_reason;
                }
            }
        }

        Ok((tool_call_chunks, finish_reason))
    }
}

// ── Tool 执行 ──────────────────────────────────────────────────

impl ConversationDriver {
    /// 执行本轮所有 tool call，将结果追加到消息列表。
    async fn handle_tool_calls(
        &self,
        messages: &mut Vec<ChatCompletionRequestMessage>,
        tool_call_chunks: &[ChatCompletionMessageToolCall],
        _client: &Client<OpenAIConfig>,
    ) -> Result<()> {
        tracing::info!("AI 请求 {} 个 tool calls", tool_call_chunks.len());

        // 添加 assistant message（含 tool_calls）
        messages.push(
            ChatCompletionRequestAssistantMessageArgs::default()
                .tool_calls(tool_call_chunks.to_vec())
                .build()?
                .into(),
        );

        for tc in tool_call_chunks {
            tracing::info!("执行 tool: {} (id: {})", tc.function.name, tc.id);

            let result = match tc.function.name.as_str() {
                "describe_model_schema" => handle_describe_schema(&self.config),
                other => {
                    let args: Value =
                        serde_json::from_str(&tc.function.arguments).unwrap_or(Value::Null);
                    match self.mcp_client.call_tool(other, args).await {
                        Ok(text) => format_tool_result(other, &text),
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

            if self.cancel.is_cancelled() {
                tracing::info!("收到取消信号（tool call 后），终止分析");
                return Ok(());
            }
        }

        Ok(())
    }
}

// ── Prompt 构建 ────────────────────────────────────────────────

/// 从 prompts/system-prompt.md 加载模板并替换占位符。
/// 占位符：`{database}` → 数据库名，`{collections}` → 集合名列表（每行一个 `\`name\``）。
fn build_system_prompt(config: &Config) -> String {
    let template = std::fs::read_to_string("prompts/system-prompt.md")
        .unwrap_or_else(|e| {
            tracing::error!("无法读取 prompts/system-prompt.md: {}", e);
            String::new()
        });

    let cols: Vec<String> = config
        .schema
        .collections
        .keys()
        .map(|c| format!("\t`{}`", c))
        .collect();

    template
        .replace("{database}", &config.mcp.database)
        .replace("{collections}", &cols.join("\n"))
}

// ── Tool 定义 & 格式化 ─────────────────────────────────────────

fn mcp_tools_to_openai(mcp_tools: &[McpTool], config: &Config) -> Vec<ChatCompletionTool> {
    mcp_tools
        .iter()
        .filter(|t| t.name == "find" || t.name == "aggregate" || t.name == "count")
        .map(|t| {
            let mut params = t.input_schema.clone();
            if let Some(obj) = params.as_object_mut() {
                if let Some(props) = obj.get_mut("properties") {
                    if let Some(props_obj) = props.as_object_mut() {
                        if let Some(col_schema) = props_obj.get_mut("collection") {
                            if let Some(col_obj) = col_schema.as_object_mut() {
                                let names = config.schema.collections.keys();
                                col_obj.insert(
                                    "description".into(),
                                    serde_json::json!(format!(
                                        "集合名称。数据库 `{}` 中可用集合: {}",
                                        config.mcp.database,
                                        names
                                            .map(|n| format!("`{}`", n))
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    )),
                                );
                            }
                        }
                    }
                }
            }
            ChatCompletionTool {
                r#type: ChatCompletionToolType::Function,
                function: FunctionObject {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: Some(params),
                    strict: None,
                },
            }
        })
        .collect()
}

fn describe_model_schema_tool() -> ChatCompletionTool {
    ChatCompletionTool {
        r#type: ChatCompletionToolType::Function,
        function: FunctionObject {
            name: "describe_model_schema".into(),
            description: Some(
                "获取数据库集合的完整结构信息，包括所有字段的中文含义和 propertySet 分组说明。在查询数据前应调用此工具了解数据结构。"
                    .into(),
            ),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            })),
            strict: None,
        },
    }
}

fn handle_describe_schema(config: &Config) -> String {
    let mut desc = schema_description_text(config);
    let list = config
        .schema
        .collections
        .keys()
        .map(|n| format!("`{}`", n))
        .collect::<Vec<_>>()
        .join(", ");
    desc.push_str(&format!(
        "\n\n## 查询提示\n\
        - 数据库: `{}`\n\
        - 可用集合: {}\n\
        - 查询时务必在 find/aggregate 中指定 collection 参数\n\
        - 过滤模型 ID: {{\"guid\": {{\"$in\": [\"GUID\"]}}}}\n\
        - 过滤属性维度: {{\"propertySet.paramGroupId\": \"分组名\"}}\n\
        - 大数据量时按 _id 范围分段查询，每批 ≤ 200 条，用 $gte/$lt 游标翻页，禁用 skip\n\
        - MongoDB _id 是 ObjectId 类型，查询格式: {{\"_id\": {{\"$gte\": {{\"$oid\": \"xxxxxxxxxxxxxxxxxxxxxxxx\"}}}}}}\n",
        config.mcp.database, list,
    ));
    desc
}

fn schema_description_text(config: &Config) -> String {
    let mut desc = String::new();
    for (name, col) in &config.schema.collections {
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

fn format_tool_result(name: &str, raw: &str) -> String {
    let max_len = 8000;
    let content = if raw.len() > max_len {
        format!(
            "{}...（结果已截断，共 {} 字符）",
            &raw[..max_len],
            raw.len()
        )
    } else {
        raw.to_string()
    };
    match name {
        "find" | "aggregate" => match serde_json::from_str::<Value>(&content) {
            Ok(pretty) => serde_json::to_string_pretty(&pretty).unwrap_or(content),
            Err(_) => content,
        },
        _ => content,
    }
}
