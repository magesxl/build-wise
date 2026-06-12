# AGENTS.md

> **AI 编码准则**：本项目采用模块化架构。在生成、重构或修复代码时，**必须**遵守以下规范。

---

## 1. 项目概览

| 属性     | 值                                                                                    |
| -------- | ------------------------------------------------------------------------------------- |
| 项目名称 | build-wise                                                                            |
| 基础框架 | Rust **Edition 2021** / Axum **0.8** / Tokio **1** (async runtime)                     |
| 存储层   | MongoDB（通过 MCP Server 访问）                                                        |
| AI 服务  | DeepSeek API（async-openai **0.28** 适配）                                              |
| 前端     | 单页 HTML（`test.html`），SSE 流式响应                                                  |
| 架构风格 | 模块化架构（api → ai / mcp → config）                                                  |
| 模块根   | `src/main.rs`                                                                          |
| 取消机制 | `CancellationToken` + `POST /api/cancel` 中断进行中的分析任务                           |

---

## 2. 目录结构说明

```text
src/
├── main.rs                     # 应用入口：路由注册、MCP 初始化、Ctrl+C 优雅关闭
├── config.rs                   # 配置层：YAML 解析 + 环境变量覆盖 + 多集合 Schema 描述
├── api/                        # API 层
│   ├── mod.rs                  # 模块声明
│   └── chat.rs                 # POST /api/chat (SSE 流式分析) + /api/cancel (取消任务)
├── ai/                         # AI 层
│   ├── mod.rs                  # 模块声明
│   └── driver.rs               # ConversationDriver：prompt 构建、tool 注册、流式合并、tool 分发
└── mcp/                        # MCP 集成层
    ├── mod.rs                  # 模块声明
    └── client.rs               # MCP stdio 客户端：子进程管理、JSON-RPC 通信、跨平台 npx 解析

config.yaml                     # 非敏感配置（端口、日志级别、模型参数、多集合 Schema）
.env                            # 敏感信息（DEEPSEEK_API_KEY、MDB_MCP_CONNECTION_STRING）
prompts/
└── system-prompt.md            # System prompt 模板（含占位符 {database}、{collections}）
test-tailwind.html                      # 前端测试页（SSE + Markdown 渲染）
docs/
├── adr/                        # 架构决策记录（ADR）
├── PRD-建造信息AI分析系统.md    # 产品需求文档
└── design-grilling.md          # 设计评审记录
```

## 3. 命名约定

### 3.1 模块/文件命名

| 类型                | 命名模式                                   | 示例                             |
| ------------------- | ------------------------------------------ | -------------------------------- |
| 模块入口            | `mod.rs`                                   | `src/api/mod.rs`                 |
| 功能模块            | 下划线分隔，语义化                           | `deepseek.rs`、`mcp/client.rs`   |
| 配置                | `config.rs`                                | `src/config.rs`                  |
| 测试                | `#[cfg(test)] mod tests { ... }`（内联）    | 同文件底部                        |

### 3.2 函数/结构体命名

| 场景       | 命名规范                                    | 示例                                    |
| ---------- | ------------------------------------------- | --------------------------------------- |
| 结构体     | PascalCase                                  | `McpClient`、`AppState`、`SseEvent`     |
| 公开函数   | snake_case                                  | `build_system_prompt()`、`run_analysis()`|
| 异步函数   | async fn + snake_case                       | `async fn connect()`                    |
| 构建器     | `Xxx::new()` / `Xxx::build()`               | `Config::load()`                        |
| Handler    | `xxx_handler`                               | `chat_handler`                          |
| 转换/映射  | `xxx_to_yyy()`                              | `mcp_tools_to_openai()`                 |

### 3.3 Trait 实现

| 用途       | Derive 宏                                   |
| ---------- | ------------------------------------------- |
| 调试输出   | `#[derive(Debug)]`                          |
| 序列化     | `#[derive(Serialize, Deserialize)]`         |
| 克隆       | `#[derive(Clone)]`                          |

---

## 4. 快速参考：新增一个完整功能流程

1. `config.yaml` → 如需新配置项或新集合 Schema，先定义
2. `src/config.rs` → 添加对应 `Config` 结构体字段
3. `src/mcp/client.rs` → 如需新 MCP 协议操作，扩展 `McpClient`
4. `prompts/system-prompt.md` → 调整 AI 行为规范和输出格式
5. `src/ai/driver.rs` → 调整 tool 定义、流式处理或 tool 执行逻辑
6. `src/api/chat.rs` → 实现对话逻辑或新端点
7. `src/main.rs` → 注册新路由
8. `test-tailwind.html` → 如需前端变更，更新测试页

## 5. 代码生成 Checklist

AI Agent 在生成代码时，请逐项确认：

- [ ] **模块正确**：代码放在正确的模块目录下（api / ai / mcp / config）
- [ ] **可见性**：对外接口用 `pub`，内部实现保持私有
- [ ] **命名规范**：结构体名 PascalCase，函数名 snake_case，符合项目约定
- [ ] **错误处理**：使用 `anyhow::Result<T>` 作为公开 API 返回值；关键操作有 `.context()` 附加错误上下文
- [ ] **日志**：使用 `tracing::info!()` / `tracing::error!()` / `tracing::warn!()`
- [ ] **异步**：IO 操作使用 async/await + Tokio runtime
- [ ] **配置**：敏感信息从 `.env` 读取，非敏感放 `config.yaml`
- [ ] **MCP 接入**：所有 MongoDB 查询通过 `McpClient::call_tool()` 执行，不直连数据库
- [ ] **SSE 流式**：输出通过 `tokio::sync::mpsc` channel → `SseEvent` 枚举 → `Sse::new(stream)`
- [ ] **优雅关闭**：正确处理 Ctrl+C 信号和 MCP 子进程清理
- [ ] **不要引入新依赖**：优先使用 Cargo.toml 中已有的 crate

---

## 6. Agent Skills

### 6.1 分析

- **系统提示词**：模板在 `prompts/system-prompt.md`，运行时由 `build_system_prompt()` 替换占位符后注入。修改分析行为从模板入手
- **Tool Calling**：AI 自动调用 MCP 的 `find`/`aggregate`/`count` 工具查询 MongoDB，外加本地 `describe_model_schema` 工具。对话循环最多 30 轮，支持 `CancellationToken` 取消
- **Schema 描述**：`config.yaml` 的 `schema.collections` 段定义了多集合（`xEntity`/`xLevel`/`xBuilding`）的结构映射，`schema_description_text()` 生成 AI 可读文本

### 6.2 运维与修复

- **Bug 修复**：先对照 `tracing` 日志定位，检查 MCP 子进程是否正常（`McpClient::shutdown()` 含 2s 超时 + 进程树强杀）
- **配置变更**：非敏感项改 `config.yaml`，敏感项改 `.env`（模板见 `.env.example`）
- **MCP 问题**：检查 `npx mongodb-mcp-server` 是否可正常运行，确认 `MDB_MCP_CONNECTION_STRING` 正确
- **跨平台**：Windows 自动用 `npx.cmd`，Unix 用 `npx`；进程树终止 Windows 用 `taskkill /T`，Unix 用 `kill -TERM -PID`
- **取消任务**：前端可通过 `POST /api/cancel` 中断进行中的分析，服务端用 `CancellationToken` 在流式消费和 tool 执行间隙检查取消信号

## 7. 构建指令

### 本地开发
- **编译**：`cargo build`
- **运行**：`cargo run`
- **检查**：`cargo check`（快速验证语法/类型）
- **测试**：`cargo test`
- **格式化**：`cargo fmt`
- **Lint**：`cargo clippy`

### Docker
- **构建镜像**：`docker build -t build-wise .`
- **单容器运行**：`docker run -p 3000:3000 --env-file .env build-wise`
- **docker-compose 一键启动**（含 MongoDB）：`docker-compose up -d`
- **镜像说明**：多阶段构建（rust:1-alpine → alpine:3.21），含 Rust 二进制 + Node.js（供 MCP Server 运行）+ config.yaml
- **MCP 跨平台**：启动时自动检测 OS，Windows 用 `npx.cmd`，Linux/macOS 用 `npx`，无需手动配置
- **容器互联**：MongoDB 同为容器时，用 docker-compose 或 `--network` 共享网络，以容器名作为 hostname

---

## Agent skills

### Issue tracker

Issues 以本地 Markdown 文件形式存放在 `.scratch/` 目录下。详见 `docs/agents/issue-tracker.md`。

### Triage labels

使用默认标准标签（needs-triage、needs-info、ready-for-agent、ready-for-human、wontfix）。详见 `docs/agents/triage-labels.md`。

### Domain docs

单上下文仓库：`CONTEXT.md` + `docs/adr/` 均位于仓库根目录。详见 `docs/agents/domain.md`。
