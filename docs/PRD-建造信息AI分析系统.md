# PRD: 建造信息 AI 分析系统

## Problem Statement

施工现场人员需要从 BIM 模型数据（RVT/IFC 解析入库）中快速获取建造信息分析。当前方式需要人工翻阅大量属性数据，无法即时获取结构化的分析结论。用户期望：输入模型 ID，AI 自动查询数据并以制式化报告形式呈现分析结果，覆盖工程量、成本、进度、模板周转等维度。

## Solution

构建一个 AI Agent 系统。用户在前端输入模型标识，系统通过 AI（DeepSeek）智能生成 MongoDB 查询语句，经 MCP Server 执行后获取数据，再由 AI 汇总为数据报告返回前端。前端展示制式化分析内容，并在回答下方提供快捷追问按钮。

## User Stories

1. As a 施工现场工程师, I want to 输入模型 ID 获取建造信息分析, so that 我能快速了解当前施工状态
2. As a 施工现场工程师, I want to 看到以数据报告形式呈现的分析结果（表格+数字为主）, so that 我能直观地做决策
3. As a 施工现场工程师, I want to 在回答下方看到快捷追问按钮, so that 我能一键深入探查具体维度
4. As a 施工现场工程师, I want to 查看到工程量分析（面积、体积等）, so that 我能评估物料使用情况
5. As a 施工现场工程师, I want the AI 自动理解我提出的问题并按需覆盖分析维度, so that 我不会被固定格式限制
6. As a 前端开发者, I want 一个统一的 SSE 流式接口, so that 前端能实现打字机效果
7. As a 系统管理员, I want 系统在出错时展示友好错误信息, so that 用户不会被技术细节困惑
8. As a 项目经理, I want 后续能扩展更多分析维度（成本、进度、模板周转）, so that 系统能持续演进

## Implementation Decisions

### 领域模型

- **模型**：从 RVT 或 IFC 文件解析入库的 BIM 数据单元。一个建筑体可由多个模型组成。唯一标识为 `guid` 字段
- **建造信息**：模型的属性数据，存储于 MongoDB 中。业务属性集中在 `xEntity.propertySet` 数组，通过 `paramGroupId` 分组（如"工程量信息"）。多集合支持（`xEntity` 实体 + `xLevel` 楼层 + `xBuilding` 建筑），`config.yaml` `schema.collections` 配置
- **进度节点**：模型属性中描述施工进度阶段的字段，值为语义化名称（如"基础施工"、"主体结构封顶"）
- **建造信息分析**：制式化的 AI 分析输出，按固定维度组织（空间位置+进度、成本、工程量、模板脚手架周转），附带快捷追问按钮

### 项目结构

- 单体 Rust crate，模块化拆分：`config`（配置加载）、`api/chat`（HTTP 接口+SSE）、`ai/deepseek`（DeepSeek 编排+Tool 定义）、`mcp/client`（MCP stdio 客户端）
- 起步简单，后续可拆为 Cargo workspace

### 技术选型

- **Web 框架**：Axum（与 tokio 生态原生兼容，DeepSeek HTTP 调用和 MCP 子进程共享同一 runtime）
- **Rust edition**：2021（稳定）
- **AI 模型**：DeepSeek（通过 `async-openai` crate 对接，兼容 OpenAI 格式的 function calling）
- **MongoDB 接入**：不直接在业务层调 MongoDB。通过开源 MCP Server（`mongodb-mcp-server`，Node.js）封装，Axum 侧通过 stdio 子进程通信
- **MCP 通信方式**：stdio 子进程（spawn `npx mongodb-mcp-server`），手写 JSON-RPC 客户端，不用现成 MCP client 库以获得完全控制
- **配置格式**：YAML（`config.yaml`），启动时加载，语义标注映射放在配置文件中而不硬编码

### API 契约

- **端点**：单一 `POST /api/chat`，统一对话入口
- **请求体**：
  ```json
  {
    "model_ids": ["guid1", "guid2"],
    "messages": [
      {"role": "user", "content": "请分析工程量"},
      {"role": "assistant", "content": "..."}
    ]
  }
  ```
- **响应**：SSE 流式，格式为 `data: {"type":"content","text":"..."}\n\n`
- **SSE 事件类型**：`content`（文本片段）、`done`（结束）、`error`（错误信息）
- **无状态**：前端每次请求携带完整对话历史，服务端不维护会话

### AI 行为规范

- **角色**："分析模型的助手"
- **输出风格**：数据报告（表格、数字为主），非叙事分析
- **维度覆盖**：按需覆盖（用户问什么就分析什么），不强制 5 个维度全出
- **缺数据处理**：某维度无数据则跳过，不编造
- **工作方式**：每次请求先调 `describe_model_schema` Tool 了解数据结构（含查询提示+分页约束），再生成 MongoDB 查询
- **MCP 子进程**：Axum 自动 spawn `mongodb-mcp-server`，跨平台 npx 解析（Windows 用 `npx.cmd`，Unix 用 `npx`）

### Tool Calling 架构

- 当前实现：`describe_model_schema` 为本地处理的合成 Tool（不经过 MCP Server），由 Axum 直接根据配置文件返回 schema 描述
- MCP Server 暴露的通用 Tool（`find`、`aggregate`、`count`）由 Axum 透传 JSON-RPC 调用，其他 tool 被过滤
- 对话循环最多 30 轮 tool calling，超过则提示用户简化提问
- Tool 结果超过 8000 字符自动截断，防止 token 爆炸
- **支持取消**：`CancellationToken` 在每轮开始、流式消费中、tool 执行后检查取消信号

### Schema 语义标注

- 在 `config.yaml` 的 `schema.fields` 中维护集合字段→中文含义映射
- `propertySet` 的 `paramGroupId` 分组在 `schema.property_groups` 中标注
- `describe_model_schema` 返回内容由 Axum 从配置动态生成，不依赖 MCP Server

### 部署与运行

- **开发期**：`cargo run` 一行启动（Axum 自动 spawn MCP Server 子进程），单机裸跑
- **日志**：tracing + env-filter，级别可配置（`config.yaml` 的 `server.log_level`）
- **端口**：可配置（默认 3000）

### 快捷追问

- 前端硬编码追问列表，放在回答下方
- 后端只需输出分析结果，不负责生成追问
- 追问内容示例："进度是否滞后？""成本与预算对比？"

## Testing Decisions

### 测试策略

- 只测外部行为，不测实现细节
- 优先集成测试，验证端到端数据流
- Mock 外部依赖（DeepSeek API、MCP Server、MongoDB）

### 测试接缝

1. **HTTP API 集成测试**（最高缝）：
   - 启动 Axum 测试服务 + mock DeepSeek endpoint + mock MCP stdin
   - 验证 `POST /api/chat` 返回正确的 SSE 事件类型和顺序
   - 验证错误场景（API key 无效、MCP 进程崩溃、MongoDB 不可达）
2. **Tool 处理测试**：
   - `describe_model_schema`：给定配置，验证返回的 schema 描述包含所有必要的字段和分组
   - Tool 结果格式化：验证长文本截断正确
3. **SSE 格式测试**：
   - 验证 `SseEvent::to_json()` 产生符合规范的 JSON
   - 验证 Axum 的 `Event::default().data()` 包装后输出合法 SSE 格式
4. **MCP 客户端测试**：
   - 用 mock JSON-RPC 子进程验证请求/响应匹配
   - 验证子进程崩溃后错误向上传播

### 不做

- 不测 DeepSeek 实际返回内容（外部服务，不可控）
- 不测 MongoDB 实际查询结果（MCP Server 覆盖）
- 不测前端 UI

## Out of Scope

- **成本分析维度**：留待后续实现（当前仅做工程量）
- **模板和脚手架周转分析**：留待后续实现
- **进度节点分析**：留待后续实现
- **MongoDB 解析入库逻辑**（RVT/IFC → MongoDB）：不在本系统范围内，假设数据已就绪
- **前端页面开发**：仅负责后端 API，前端独立开发
- **用户认证与权限**：不在本期范围
- **性能优化与缓存**：不在本期范围
- **生产部署配置**：仅保证开发期单机运行

## Further Notes

- 领域术语表见 `CONTEXT.md`
- 如有需要记录架构权衡决策，将在 `docs/adr/` 下创建 ADR
- 配置文件 `config.yaml` 中的 `api_key` 为占位符，需替换为真实密钥
