# AI Agent 架构：Tool Calling 模式 + 无状态 API + describe_model_schema

决定让 DeepSeek 自己决定查什么数据（模式 ①），通过独立的 `describe_model_schema` Tool 让 AI 按需探查数据库结构。API 无状态，前端每次携带完整对话历史。

## 背景

用户输入模型 ID 和问题后，系统需要查询 MongoDB 数据并总结。有两种模式：AI 决定查什么 vs 业务层先查好数据再给 AI。

## 决定

- **集成模式**：DeepSeek 通过 function calling 自主决定调用哪些 MCP Tool、传递什么参数来查询数据
- **Schema 注入**：暴露独立的 `describe_model_schema` Tool，每次请求 AI 先探查数据结构，再生成查询
- **API 状态**：无状态，前端每轮请求携带完整 `messages` 数组，服务端不维护会话

## 考虑过的方案

- **模式 ② 业务层先查**：可控性好，prompt 和数据结构完全由开发者掌握，但不灵活，每新增分析维度需改代码
- **Schema 写死在 System Prompt**：零延迟，但 schema 膨胀 system prompt（消耗 token），且 BIM 字段可能随版本变化，静态注入过时风险高
- **Schema 写在 MCP Tool description 里**：简单直接，但 Schema 和 Tool 逻辑耦合，难以单独维护和更新
- **服务端 session（Redis/内存）**：减少前端传输量，但引入会话管理复杂度，且当前对话深度浅（分析+追问），状态管理的收益不大

## 后果

- AI 可灵活适配不同分析需求，新增维度只需配置，不用改代码
- MCP tools 过滤：仅暴露 `find`、`aggregate`、`count`，外加本地 `describe_model_schema`
- System prompt 从外部文件 `prompts/system-prompt.md` 加载，`{database}` 和 `{collections}` 占位符运行时替换
- 无状态简化部署和水平扩展，前端已能自行管理对话历史（追问按钮硬编码在前端）
- 最大 **30 轮** tool calling 防止无限循环（在每轮开始处检查取消信号）
- `CancellationToken` 支持，由 `POST /api/cancel` 端点触发
