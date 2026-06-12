# 思考过程流式分离：reqwest 裸调替代 async-openai typed streaming

DeepSeek 开启 thinking 模式后，流式响应 delta 中包含 `reasoning_content`（思考过程）和 `content`（正式回答）两个独立字段。但 `async-openai` crate（0.28 ~ 0.41）的 `ChatCompletionStreamResponseDelta` 结构体没有 `reasoning_content` 字段，且无 `#[serde(deny_unknown_fields)]`，导致该字段被静默丢弃。为捕获思考过程，改为 reqwest 裸调 + 手动 SSE/JSON 解析。

参考了 pi coding agent（`@earendil-works/pi-ai`）的 `openai-completions.js` 实现——Node.js `openai` SDK 返回动态对象，直接读取 `delta.reasoning_content` 无类型障碍。Rust 侧需要等价地绕过强类型层。

## 决定

### 后端协议

新增 `SseEvent::Thinking(String)` 变体，与 `Content` 平级：

```
thinking → {"type":"thinking","text":"..."}
content  → {"type":"content","text":"..."}
done     → {"type":"done"}
error    → {"type":"error","text":"..."}
```

### 流式采集

- **HTTP 层**：`reqwest` 直接 POST 到 DeepSeek `/chat/completions`，`Accept: text/event-stream`
- **请求体**：用 `async-openai` 的 `CreateChatCompletionRequestArgs` 构建消息和 tools，`serde_json::to_value()` 序列化后注入 `"thinking": {"type": "enabled"}` 和 `"stream": true`
- **SSE 解析**：逐行读取 `data: {...}`，`serde_json::Value` 解析，提取：
  - `choices[0].delta.reasoning_content` → `SseEvent::Thinking`
  - `choices[0].delta.content` → `SseEvent::Content`
  - `choices[0].delta.tool_calls` → 手动按 index 累积为 `ChatCompletionMessageToolCall`
  - `choices[0].finish_reason` → 字符串映射 `FinishReason` 枚举
- **`[DONE]` 标记**：识别并正常结束流

### 前端渲染

- **思考区域**：`<details open>` 包裹，标题 `💭 思考过程`，内容区灰底（`#f3f4f6`）+ 紫色左边框 + 12px 灰色纯文本（不跑 Markdown）。在新消息发送时重建
- **定位**：插入在 AI 头部行和回答气泡之间（外层 wrap 的直接子级），`margin-left: 52px`（avatar 40px + ml-3 12px），左边缘与回答气泡精确对齐
- **回答气泡**：`addMsgBubble('assistant')` 创建后立即 `aiDiv.style.display = 'none'` 仅隐藏白色气泡本身，**flex row 中的 AI 头像保持可见**。`content` 事件到达时渲染到隐藏气泡中（后台累积），用户不可见
- **完成时展示**：`done` 事件到达时同时做两件事——`display: ''` 显示回答气泡（含完整 Markdown 内容），0.5s CSS `max-height/opacity` 动画折叠思考区 → `details.open = false`
- **异常时展示**：`error` 事件或 `catch` 时显示回答气泡 + 错误信息
- **流式效果**：回答文本丢失了打字机效果（全程隐藏到 `done`），但消除了 tool calling 多轮导致的思考区反复展开/折叠的视觉闪烁

### 状态管理

- `thinkingEl`、`thinkingText` 声明在 `while(true)` 外层，跨 SSE chunk 保持
- 每次 `send()` 调用重建局部变量，自动重置
- 无需 `collapseTimer`（折叠逻辑内嵌在 `done` 的 `setTimeout` 中，无需跨 chunk 持有的句柄）

## 考虑过的方案

### 升级 async-openai

0.41.0 仍未包含 `reasoning_content` 字段，此路不通。

### fork/patch async-openai

加字段需改上游类型定义并维护 fork，收益不如直接裸调。

### 系统提示词分隔符

让模型在 `content` 中输出 `...` / `...` 包裹思考，后端解析分离。但依赖模型遵守约定，不可靠。

### 折叠时机（3 轮迭代）

| 方案 | 描述 | 结果 |
|------|------|------|
| v1：首 `content` 后 0.5s 折叠 | 思考区在正式回答开始时收起 | tool calling 多轮导致交替展开/折叠，视觉突兀 |
| v2：立刻折叠、延迟折叠、done 后折叠对比 | 逐一评估 | 全部无法解决多轮交替问题 |
| v3（最终）：`done` 时折叠 | 思考区全程展开，`done` 时一次性收起 + 显示回答 | 消除闪烁，牺牲回答流式打字机效果 |

### 回答气泡显示时机

初版在首 `content` 时显示气泡，导致多轮 tool calling 中空框闪现。改为 `done` 时显示（与折叠同步），思考期间气泡完全隐藏。`error`/`catch` 时也显示气泡确保错误可见。

### 思考区 DOM 定位

- 初版：flex row 平级插入（左边缘对齐头像，而非气泡），修正为 `margin-left: 52px` 对齐回答气泡
- 隐藏气泡时使用 `aiDiv.parentElement.style.display = 'none'` 导致整个 flex row（含 AI 头像）消失，改为 `aiDiv.style.display = 'none'` 仅隐藏白色气泡，头像在思考期间保持可见

## 后果

- `reqwest` 成为新依赖
- `stream_and_merge()` 从 ~50 行 typed 流式逻辑变为 ~120 行手动 SSE 解析
- 失去 `async-openai` 的 typed error 处理、自动重试等便利，需自行处理 HTTP 错误和 JSON 解析异常
- 后端新增 `Thinking` 事件类型，前端分两路处理 `thinking`/`content`，复杂度略增
- 前端 UX 取舍：牺牲回答流式打字机效果，换取零闪烁的思考区体验
- 后续若 `async-openai` 加入 `reasoning_content` 支持，可考虑回迁 typed streaming
