# SSE 接口规范：统一格式 + 流式 + 单一端点

决定 Axum 对前端输出统一的 SSE 格式，隐藏背后的 DeepSeek tool calling 过程。单一路径 `POST /api/chat` 处理全部对话。

## 背景

DeepSeek 的流式 SSE 包含 `delta`、`tool_calls` 等原始 chunk，前端需处理 function calling 的中间状态（tool call 参数的分片累积等）。需要决定是透传还是封装。

## 决定

- **SSE 格式**：Axum 层做统一格式转换，前端只看到三种事件类型：`content`（文本片段）、`done`（结束）、`error`（错误）
- **流式**：支持 SSE 流式输出（打字机效果）
- **端点**：单一 `POST /api/chat`，不按场景拆分路径
- **追问**：前端硬编码追问按钮，放在回答下方（方案 A 风格）

## 考虑过的方案

- **透传 DeepSeek 原始 SSE**：实现最简单，但前端需处理 tool_calls 分片累积逻辑，增加前端复杂度，且暴露了后端 AI 实现细节
- **按场景分路径**（如 `/api/analysis/build-info`）：路径更语义化，但当前只有一个分析场景，过度设计

## 后果

- 前端只需消费三种事件，无需理解 function calling
- tool calling 过程（查询 schema → 查询数据 → 总结）对前端完全透明，用户只看到 AI "思考"和"回答"
- SSE 格式固定，前端可稳定解析
- 未来如需暴露 tool calling 中间状态（如"正在查询工程量数据..."），可在 `content` 事件中携带 `status` 子类型扩展，无需改协议
