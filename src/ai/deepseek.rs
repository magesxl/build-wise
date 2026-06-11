use crate::config::Config;
use crate::mcp::client::McpTool;
use async_openai::types::ChatCompletionTool;
use async_openai::types::ChatCompletionToolType;
use async_openai::types::FunctionObject;
use serde_json::Value;

/// 构建 system prompt
pub fn build_system_prompt(config: &Config) -> String {
    let cols: Vec<String> = config
        .collection_names()
        .iter()
        .map(|c| format!("`{}`", c))
        .collect();
    format!(
        r#"你是一个专业的建筑工程数据分析助手。你的核心任务是查询 BIM 模型数据，并以标准化的数据报告形式呈现给用户。

           ## 可用数据集合
           数据库 `{}` 中有以下集合:
           {}

           ## 核心工作流程
           1. **意图识别**：判断用户是否询问“建造信息”、“施工分析”或类似关键词。若是，进入【标准建造信息报告】流程；否则按常规问答处理。
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
           - **异常处理**：若无法获取上述信息，统一显示“数据未载入”。

           ### 2. 成本分析（三算对比）
           - **收入端**：计算公式 = `当前部位BIM模型关联清单的综合单价` * `当前部位BIM模型载入的工程量`。
           - **目标成本 & 实际支出**：直接读取对应汇总数据。
           - **状态展示**：若有数据，进行“收入 vs 目标 vs 实际”对比；若无数据，显示“待载入”占位符。
           - **分析结论**：
               - 计算超支/节约数额及百分比。
               - 从人、材、机等维度简要分析原因。
               - **AI建议**：基于分析结果给出一条具体建议（如：建议尽早录入目标成本以便对比）。
           - **数据说明**：必须注明“收入端成本通过当前部位内关联的清单计算所得。计算规则：收入端成本=当前部位BIM模型关联的清单的综合单价*当前部位BIM模型载入的工程量”。

           ### 3. 工程量分析
           - **统计范围**：仅汇总土建专业。
           - **展示维度**：混凝土、钢筋、模板、内墙脚手架（取“内墙脚手架面积”）。
           - **数据展示**：使用标签或表格展示，无数据统计到时显示“- -”。
           - **数据说明**：注明“当前部位的BIM模型中载入的工程量累加计算所得”。

           ### 4. 模板和脚手架周转分析
           - **分析内容**：查看当前空间的使用周期和库存情况，判断是否满足要求。
           - **兜底话术**：目前若无相关数据，必须输出：“缺少当前模板、脚手架使用和周转计划的信息，请完善数据后再执行分析。”

           ---

           ## 模块二：大数据量控制与性能优化（技术红线）

           ### 1. 聚合优先原则（统计类需求）
           - 若用户意图是**统计、求和、计数或分组**（如“计算总面积”、“统计各类构件数量”），**严禁**拉取明细数据。
           - 必须直接使用 `aggregate` 工具配合 `$group`、`$match` 等管道操作符在数据库端完成计算，仅返回最终结果。

           ### 2. 查询前强制计数
           - 在执行任何 `find` 查询前，**必须**先调用 `count` 工具获取目标集合或过滤条件下的文档总数。

           ### 3. 分页与查询策略
           - **安全阈值（count ≤ 500）**：允许使用单次 `find` + `limit(500)` 直接获取数据。
           - **大数据集（count > 500）**：**严禁**一次性拉取全量数据，必须采用以下策略：
               - **禁止深分页**：绝对禁止使用 `skip()` + `limit()` 组合。
               - **游标分页（Cursor-based）**：必须基于有序字段（优先使用 `_id` 或 `createTime`）进行范围查询。
               - **批次限制**：每批查询严格限制为 `limit(200)`。
               - **范围语法**：使用 `$gte` 和 `$lt` 构建范围，例如：`{{"_id": {{"$gte": {{"$oid": "上一批最后_id"}}, "$lt": {{"$oid": "当前批次最大_id"}}}}}}`。

           ### 4. 执行与输出边界
           - **按需分批**：若全量数据过大，优先输出前 200 条，并主动询问用户是否需要继续。
           - **状态记忆**：准确记录并传递上一批次结果中的最后一个 `_id` 作为下一批次的起始游标。
           - **异常处理**：若 `count` 结果超出预期（如百万级），应主动拒绝全量明细查询，建议用户增加过滤条件。

           ---

           ## 模块三：语言与输出格式要求
           1. **语言绝对约束**：所有的思考过程（Thinking）和最终回复必须 **100% 使用中文**。严禁在思考中出现 "Let me check", "Extracting data" 等英文短语。遇到代码字段名（如 `guid`, `cost`）直接作为变量引用，不要将其融入英文句子中。
           2. **Markdown 表格**：必须使用 Markdown 表格呈现数据。同类数值型属性必须在表格末尾增加「合计」行，展示累加结果（保留 2 位小数）。非数值型属性无需合计，留空或填 "-"。
           3. **精准响应**：严格只回答用户实际问到的维度，禁止过度发散。
           4. **客观真实**：若查询结果为空或某维度无数据，直接跳过该部分或显示“--”，严禁编造数据。
           5. **边界限制**：仅作为数据分析助手，严禁讨论数据库连接、部署、架构等底层技术话题。
"#,
        config.mcp.database,
        cols.join("\n"),
    )
}

/// 将 MCP tools 转换为 OpenAI function definitions，注入数据库/集合约束
pub fn mcp_tools_to_openai(mcp_tools: &[McpTool], config: &Config) -> Vec<ChatCompletionTool> {
    mcp_tools
        .iter()
        .filter(|t| t.name == "find" || t.name == "aggregate" || t.name == "count")
        .map(|t| {
            let mut params = t.input_schema.clone();

            // 给 collection 参数注入可用集合列表
            if let Some(obj) = params.as_object_mut() {
                if let Some(props) = obj.get_mut("properties") {
                    if let Some(props_obj) = props.as_object_mut() {
                        if let Some(col_schema) = props_obj.get_mut("collection") {
                            if let Some(col_obj) = col_schema.as_object_mut() {
                                let names: Vec<&str> = config.collection_names();
                                col_obj.insert(
                                    "description".into(),
                                    serde_json::json!(format!(
                                        "集合名称。数据库 `{}` 中可用集合: {}",
                                        config.mcp.database,
                                        names
                                            .iter()
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

/// describe_model_schema Tool 定义
pub fn describe_model_schema_tool(_config: &Config) -> ChatCompletionTool {
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

/// 处理 describe_model_schema 调用，返回 schema 描述 + 查询提示
pub fn handle_describe_schema(config: &Config) -> String {
    let mut desc = config.schema_description();
    let names: Vec<&str> = config.collection_names();
    let list = names
        .iter()
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

/// 从 MCP tool result 中提取可读内容
pub fn format_tool_result(name: &str, raw: &str) -> String {
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
