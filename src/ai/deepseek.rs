use crate::config::Config;
use crate::mcp::client::McpTool;
use async_openai::types::ChatCompletionTool;
use async_openai::types::ChatCompletionToolType;
use async_openai::types::FunctionObject;
use serde_json::Value;

/// 构建 system prompt
pub fn build_system_prompt(config: &Config) -> String {
    let cols: Vec<String> = config.collection_names().iter().map(|c| format!("`{}`", c)).collect();
    format!(
        r#"你是一个建造信息分析助手。你的任务是查询 BIM 模型数据并以数据报告形式呈现。

## 可用数据集合
数据库 `{}` 中有以下集合:
{}

## 工作流程
1. 首先调用 describe_model_schema 工具了解各集合的数据结构
2. 然后根据需要调用 find 工具，指定 collection 参数查询对应的集合
3. 最后将查询结果整理为数据报告

## 输出格式要求
- 必须使用 Markdown 表格呈现数据
- 表格至少包含以下列：构件名称（name）、属性名（paramName）、属性值（paramValue）、单位
- 同类数值型属性必须在表格末尾计算并展示「合计」行
- 合计行对面积、体积等数值累加求和（去掉单位后计算，结果保留 2 位小数）
- 非数值属性（如分类文本）不需要合计

## 回答要求
- 只回答用户实际问到的维度，不主动展开无关内容
- 某个维度若无数据，直接跳过，不要编造
- 严禁讨论数据库连接、部署等技术话题，只做数据分析
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
                                    serde_json::json!(
                                        format!("集合名称。数据库 `{}` 中可用集合: {}",
                                            config.mcp.database,
                                            names.iter().map(|n| format!("`{}`", n)).collect::<Vec<_>>().join(", "))
                                    ),
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
    let list = names.iter().map(|n| format!("`{}`", n)).collect::<Vec<_>>().join(", ");
    desc.push_str(&format!(
        "\n\n## 查询提示\n\
        - 数据库: `{}`\n\
        - 可用集合: {}\n\
        - 查询时务必在 find/aggregate 中指定 collection 参数\n\
        - 过滤模型 ID: {{\"guid\": {{\"$in\": [\"GUID\"]}}}}\n\
        - 过滤属性维度: {{\"propertySet.paramGroupId\": \"分组名\"}}\n",
        config.mcp.database, list,
    ));
    desc
}

/// 从 MCP tool result 中提取可读内容
pub fn format_tool_result(name: &str, raw: &str) -> String {
    let max_len = 8000;
    let content = if raw.len() > max_len {
        format!("{}...（结果已截断，共 {} 字符）", &raw[..max_len], raw.len())
    } else {
        raw.to_string()
    };

    match name {
        "find" | "aggregate" => {
            match serde_json::from_str::<Value>(&content) {
                Ok(pretty) => serde_json::to_string_pretty(&pretty).unwrap_or(content),
                Err(_) => content,
            }
        }
        _ => content,
    }
}
