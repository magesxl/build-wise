# 技术栈选型：Rust + Axum + async-openai

决定使用 Axum 作为 Web 框架，`async-openai` crate 对接 DeepSeek，Rust 单体 crate 模块化结构起步。

## 背景

项目需要 HTTP 服务（SSE 流式响应）、AI 模型调用（DeepSeek function calling）、子进程管理（MCP Server）。所有组件需要共享同一个 async runtime。

## 决定

- **Web 框架**：Axum（tokio 原生，生态最活跃）
- **AI SDK**：`async-openai`（原生支持 function calling + streaming，API 兼容 DeepSeek）
- **项目结构**：单体 crate，模块拆分（`config`/`api`/`ai`/`mcp`），后续可拆 workspace

## 考虑过的方案

- **Actix-web**：性能好但独立 runtime，与 tokio 生态的 MongoDB driver 和 async-openai 不共享 runtime，需要桥接
- **Poem**：API 友好但社区规模不如 Axum
- **reqwest 裸调 DeepSeek**：完全控制但需手动处理 streaming、tool calling、重试等，开发量大且容易出错
- **rig / langchain-rust**：高层 Agent 框架，但抽象过重，tool calling 生命周期不如 async-openai 透明

## 后果

- Axum + async-openai 共享 tokio runtime，MCP 子进程也在同一 runtime 管理，无 runtime 桥接开销
- `async-openai` 后续如升级，API 变化可控（供应商无关的抽象层）
- 单体 crate 起步降低初期复杂度，模块边界已预留（`api/`、`ai/`、`mcp/`），拆 workspace 时只需加 `Cargo.toml` 和路径调整
