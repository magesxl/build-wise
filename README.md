# build-wise

建造信息 AI 分析系统 — 基于 Rust / Axum / DeepSeek / MongoDB 的智能分析服务。

[![CI](https://github.com/earendil-works/build-wise/actions/workflows/ci.yml/badge.svg)](https://github.com/earendil-works/build-wise/actions/workflows/ci.yml)
[![Docker](https://github.com/earendil-works/build-wise/actions/workflows/docker.yml/badge.svg)](https://github.com/earendil-works/build-wise/actions/workflows/docker.yml)

## 技术栈

| 层级     | 技术                                      |
| -------- | ----------------------------------------- |
| 运行时   | Rust Edition 2021 / Tokio 1 (async)       |
| Web 框架 | Axum 0.8                                  |
| AI 服务  | DeepSeek API (async-openai 0.28)          |
| 数据库   | MongoDB (通过 MCP Server 访问)             |
| 前端     | 单页 HTML + SSE 流式响应                   |

## 快速开始

### 前置条件

- Rust 1.80+
- Node.js 18+（MCP Server 运行环境）
- MongoDB 实例

### 本地开发

```bash
# 复制环境变量模板
cp .env.example .env
# 编辑 .env，填入 DEEPSEEK_API_KEY 和 MDB_MCP_CONNECTION_STRING

# 编译
cargo build

# 运行
cargo run

# 检查
cargo check

# 测试
cargo test

# 格式化
cargo fmt

# Lint
cargo clippy
```

服务默认监听 `http://localhost:3000`，前端测试页为 `test.html`。

### Docker

```bash
# 构建镜像
docker build -t build-wise .

# 单容器运行
docker run -p 3000:3000 --env-file .env build-wise

# docker-compose 一键启动（含 MongoDB）
docker-compose up -d
```

## CI/CD

本项目使用 GitHub Actions 自动化流水线。

### CI（持续集成）

`.github/workflows/ci.yml` — 手动触发（GitHub Actions 页面 `workflow_dispatch`）：

- **Rustfmt** — 代码风格检查
- **Clippy** — 静态分析，zero-warning 门禁
- **Build & Test** — release 编译 + 全量测试

### Docker 镜像发布

`.github/workflows/docker.yml` — 推送 `v*.*.*` tag 或手动触发：

- 构建多阶段 Docker 镜像
- 推送至 GitHub Container Registry (`ghcr.io`)
- 自动生成语义版本标签

## 项目结构

```text
src/
├── main.rs          # 应用入口：路由注册、MCP 初始化、Ctrl+C 优雅关闭
├── config.rs        # 配置层：YAML 解析 + 环境变量覆盖 + 多集合 Schema
├── api/
│   ├── mod.rs       # 模块声明
│   └── chat.rs      # POST /api/chat (SSE 流式) + /api/cancel (取消任务)
├── ai/
│   ├── mod.rs
│   └── driver.rs    # ConversationDriver：prompt、tool 注册、流式合并、tool 分发
└── mcp/
    ├── mod.rs
    └── client.rs    # MCP stdio 客户端：JSON-RPC 通信、跨平台 npx 解析
prompts/
└── system-prompt.md # System prompt 模板（占位符动态替换）
```

## 许可证

MIT
