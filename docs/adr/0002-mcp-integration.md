# MCP 集成模式：stdio 子进程 + 开源 MongoDB MCP Server

决定通过开源 `mongodb-mcp-server`（Node.js）作为 MCP Server，Axum 通过 stdio 子进程方式与之通信，手写 JSON-RPC 客户端。

## 背景

需要从 MongoDB 查询 BIM 模型数据，但不希望业务代码直接依赖 MongoDB driver。MCP（Model Context Protocol）提供了标准的 Tool 抽象，AI Agent 通过 function calling 触发数据查询。

## 决定

- **MCP Server 选型**：`mongodb-mcp-server`（Node.js/TypeScript），支持 `find`、`aggregate`、`count`、`listCollections`、`getCollectionSchema`
- **通信方式**：stdio 子进程（spawn `npx mongodb-mcp-server`），JSON-RPC over stdin/stdout
- **MCP Client 实现**：手写 Rust 客户端，不依赖 `mcp-client` crate

## 考虑过的方案

- **HTTP/SSE 远程调用**：跨机器部署灵活，但多一跳网络延迟，且需要管理 MCP Server 的独立端口和进程生命周期
- **内嵌（同进程）**：零 IPC 开销、单一二进制，但耦合在同一进程（崩一起崩），不能被其他 Agent 复用，且 MCP Server 只能用 Rust 写
- **`mcp-client` crate**：封装了 transport 层，但生态较新、成熟度不确定，手写控制更放心且代码量不大
- **Python `mcp-mongodb`**：官方 repo 的 server，但功能不如 `mongodb-mcp-server` 丰富

## 后果

- MCP Server 独立进程，可被多个 Agent 复用
- 进程隔离：MCP Server 崩溃不拖垮 Axum
- 部署需打包两个运行时（Rust 二进制 + Node.js `mongodb-mcp-server`）
- 子进程生命周期管理（spawn、health check、graceful shutdown）需在 Axum 中实现
- stdio JSON-RPC 序列化有微小 IPC 开销，但相比 MongoDB 查询延迟可忽略
