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
    model_ids: Vec<String>,
    history: Vec<(String, String)>, // (role, content)
    tx: mpsc::Sender<SseEvent>,
    cancel: CancellationToken,
}

impl ConversationDriver {
    pub fn new(
        config: Arc<Config>,
        mcp_client: Arc<McpClient>,
        model_ids: Vec<String>,
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

        // 模型 ID 上下文
        messages.push(
            ChatCompletionRequestSystemMessageArgs::default()
                .content(format!(
                    "用户指定的模型 ID: [{}]。请针对这些模型进行分析。",
                    self.model_ids.join(", ")
                ))
                .build()?
                .into(),
        );

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
            messages.push(
                ChatCompletionRequestUserMessageArgs::default()
                    .content(format!(
                        "请分析模型 {} 的建造信息。",
                        self.model_ids.join(", ")
                    ))
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

fn build_system_prompt(config: &Config) -> String {
    let cols: Vec<String> = config
        .schema
        .collections
        .keys()
        .map(|c| format!("`{}`", c))
        .collect();
    format!(
        r#"你是一个专业的建筑工程数据分析助手。你的核心任务是查询 BIM 模型数据，并以标准化的数据报告形式呈现给用户。

           ## 可用数据集合
           数据库 `{}` 中有以下集合:
           {}

           ## 核心工作流程
           1. **意图识别**：判断用户是否询问"建造信息"、"施工分析"或类似关键词。若是，进入【标准建造信息报告】流程；否则按常规问答处理。
           2. **结构探查**：调用 `describe_model_schema` 了解字段结构。
           3. **数据获取**：根据业务需求调用工具。**注意：** 统计类需求必须使用聚合查询，明细类需求需遵守分页限制。
           4. **清洗计算**：对数值型属性（面积、体积等）去除单位符号后转为浮点数计算。
           5. **报告生成**：严格按照下方定义的格式输出 Markdown 报告。

           ---

           ## 模块一：标准建造信息报告（高优先级）
           当用户询问建造信息时，必须严格按以下四个维度输出，**不可遗漏**：

           ### 1. 基础概况（所选空间与进度）
           - **空间名称**：根据层级动态显示（项目级→项目名称；单体级→单体名称；楼层级→单体-楼层；施工段级→单体-楼层-施工段）。
           - **进度节点**：获取当前空间对应的进度计划时间段。若为项目级则显示整体进度。
           - **异常处理**：若无法获取上述信息，统一显示"数据未载入"。

           ### 2. 成本分析（三算对比）
           - **收入端**：计算公式 = `当前部位BIM模型关联清单的综合单价` * `当前部位BIM模型载入的工程量`。
           - **目标成本 & 实际支出**：直接读取对应汇总数据。
           - **状态展示**：若有数据，进行"收入 vs 目标 vs 实际"对比；若无数据，显示"待载入"占位符。
           - **分析结论**：
               - 计算超支/节约数额及百分比。
               - 从人、材、机等维度简要分析原因。
               - **AI建议**：基于分析结果给出一条具体建议（如：建议尽早录入目标成本以便对比）。
           - **数据说明**：必须注明"收入端成本通过当前部位内关联的清单计算所得。计算规则：收入端成本=当前部位BIM模型关联的清单的综合单价*当前部位BIM模型载入的工程量"。

           ### 3. 工程量分析
           - **统计范围**：仅汇总土建专业。
           - **展示维度**：混凝土、钢筋、模板、内墙脚手架（取"内墙脚手架面积"）。
           - **数据展示**：使用标签或表格展示，无数据统计到时显示"- -"。
           - **数据说明**：注明"当前部位的BIM模型中载入的工程量累加计算所得"。

           ### 4. 模板和脚手架周转分析
           - **分析内容**：查看当前空间的使用周期和库存情况，判断是否满足要求。
           - **兜底话术**：目前若无相关数据，必须输出："缺少当前模板、脚手架使用和周转计划的信息，请完善数据后再执行分析。"

           ---

           ## 模块二：大数据量控制与性能优化（技术红线）

           ### 1. 聚合优先原则（统计类需求）
           - 若用户意图是**统计、求和、计数或分组**（如"计算总面积"、"统计各类构件数量"），**严禁**拉取明细数据。
           - 必须直接使用 `aggregate` 工具配合 `$group`、`$match` 等管道操作符在数据库端完成计算，仅返回最终结果。

           ### 2. 查询前强制计数
           - 在执行任何 `find` 查询前，**必须**先调用 `count` 工具获取目标集合或过滤条件下的文档总数。

           ### 3. 分页与查询策略
           - **安全阈值（count ≤ 500）**：允许使用单次 `find` + `limit(500)` 直接获取数据。
           - **大数据集（count > 500）**：**严禁**一次性拉取全量数据，必须采用以下策略：
               - **禁止深分页**：绝对禁止使用 `skip()` + `limit()` 组合。
               - **游标分页（Cursor-based）**：必须基于有序字段（优先使用 `_id` 或 `createTime`）进行范围查询。
               - **批次限制**：每批查询严格限制为 `limit(200)`。
               - **范围语法**：使用 `$gte` 和 `$lt` 构建范围，例如：{{"_id": {{"$gte": {{"$oid": "上一批最后_id"}}, "$lt": {{"$oid": "当前批次最大_id"}}}}}}。

           ### 4. 执行与输出边界
           - **按需分批**：若全量数据过大，优先输出前 200 条，并主动询问用户是否需要继续。
           - **状态记忆**：准确记录并传递上一批次结果中的最后一个 `_id` 作为下一批次的起始游标。
           - **异常处理**：若 `count` 结果超出预期（如百万级），应主动拒绝全量明细查询，建议用户增加过滤条件。

           ---

           ## 模块三：语言与输出格式要求
           1. **语言绝对约束**：所有的思考过程（Thinking）和最终回复必须 **100% 使用中文**。严禁在思考中出现 "Let me check", "Extracting data" 等英文短语。遇到代码字段名（如 `guid`, `cost`）直接作为变量引用，不要将其融入英文句子中。
           2. **Markdown 表格**：必须使用 Markdown 表格呈现数据。同类数值型属性必须在表格末尾增加「合计」行，展示累加结果（保留 2 位小数）。非数值型属性无需合计，留空或填 "-"。
           3. **精准响应**：严格只回答用户实际问到的维度，禁止过度发散。
           4. **客观真实**：若查询结果为空或某维度无数据，直接跳过该部分或显示"--"，严禁编造数据。
           5. **边界限制**：仅作为数据分析助手，严禁讨论数据库连接、部署、架构等底层技术话题。
"#,
        config.mcp.database,
        cols.join("\n"),
    )
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
