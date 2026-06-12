# 思考过程流式分离：reqwest 裸调替代 async-openai typed streaming

DeepSeek 开启 thinking 模式后，流式响应 delta 中包含 `reasoning_content`（思考过程）和 `content`（正式回答）两个独立字段。但 `async-openai` crate（0.28 ~ 0.41）的 `ChatCompletionStreamResponseDelta` 结构体没有 `reasoning_content` 字段，且无 `#[serde(deny_unknown_fields)]`，导致该字段被静默丢弃。为捕获思考过程，改为 reqwest 裸调 + 手动 SSE/JSON 解析。

参考了 pi coding agent（`@earendil-works/pi-ai`）的 `openai-completions.js` 实现——Node.js `openai` SDK 返回动态对象，直接读取 `delta.reasoning_content` 无类型障碍。Rust 侧需要等价地绕过强类型层。

## 决定

### 后端协议

新增 `SseEvent::Thinking(String)` 和 `SseEvent::ThinkingDone` 变体，与 `Content` 平级：

```
thinking      → {"type":"thinking","text":"..."}
thinking_done → {"type":"thinking_done"}
content       → {"type":"content","text":"..."}
done          → {"type":"done"}
error         → {"type":"error","text":"..."}
```

`ThinkingDone` 由后端在**最终轮（`finish_reason == Stop`）流末尾自动插入**，标记思考结束。非最终轮（`ToolCalls`）不发送，思考区在多轮间持续展开。

### 流式采集

- **HTTP 层**：`reqwest` 直接 POST 到 DeepSeek `/chat/completions`，`Accept: text/event-stream`
- **请求体**：用 `async-openai` 的 `CreateChatCompletionRequestArgs` 构建消息和 tools，`serde_json::to_value()` 序列化后注入 `"thinking": {"type": "enabled"}` 和 `"stream": true`
- **SSE 解析**：逐行读取 `data: {...}`，`serde_json::Value` 解析，提取：
  - `choices[0].delta.reasoning_content` → `SseEvent::Thinking`
  - `choices[0].delta.content` → `SseEvent::Content`
  - `choices[0].delta.tool_calls` → 手动按 index 累积为 `ChatCompletionMessageToolCall`
  - `choices[0].finish_reason` → 字符串映射 `FinishReason` 枚举
- **边界检测**：`thinking_seen` 布尔标记追踪当前轮是否出现过 `reasoning_content`。标记持续为 `true` 直至函数出口。在流末尾（`[DONE]` 标记或循环正常结束处）检查：若 `thinking_seen && finish_reason == Stop`，发送 `SseEvent::ThinkingDone`。中间轮次（`finish_reason == ToolCalls`）不发送——回避了 DeepSeek 在非最终轮也输出 text content（如"我来查一下数据库"）导致过早折叠的问题
- **`[DONE]` 标记**：识别并正常结束流

### 前端渲染

- **思考区域**：`<details open>` 包裹，标题 `💭 思考过程`，内容区灰底（`#f3f4f6`）+ 紫色左边框 + 12px 灰色纯文本（不跑 Markdown）。在新消息发送时重建
- **定位**：插入在 AI 头部行和回答气泡之间（外层 wrap 的直接子级），`margin-left: 52px`（avatar 40px + ml-3 12px），左边缘与回答气泡精确对齐
- **回答气泡**：`addMsgBubble('assistant')` 创建后立即 `aiDiv.style.display = 'none'` 仅隐藏白色气泡本身，**flex row 中的 AI 头像保持可见**
- **思考展示**：`thinking` 事件到达时创建/追加思考区域，文本跨轮累积
- **思考折叠**：`thinking_done` 事件在最终轮流末尾到达（此时回答已流式输出完毕）。触发 0.5s CSS `max-height/opacity` 动画折叠思考区 → `details.open = false`。内层 `.thinking-content` 和外层 `.thinking-box` 同时收缩，回答气泡顺滑上移
- **回答流式**：`content` 事件到达时显示回答气泡（`display: ''`），Markdown 渲染 + 闪烁光标，逐字吐出打字机效果
- **完成**：`done` 事件到达时去掉光标，渲染最终 Markdown，追加追问按钮
- **异常**：`error` 事件或 `catch` 时显示回答气泡 + 错误信息

### 状态管理

- `thinkingEl`、`thinkingText` 声明在 `while(true)` 外层，跨 SSE chunk 保持
- 每次 `send()` 调用重建局部变量，自动重置
- 后端 `thinking_seen` 标记在每轮 `stream_and_merge()` 内局部追踪，自动重置

## 考虑过的方案

### 升级 async-openai

0.41.0 仍未包含 `reasoning_content` 字段，此路不通。

### fork/patch async-openai

加字段需改上游类型定义并维护 fork，收益不如直接裸调。

### 系统提示词分隔符

让模型在 `content` 中输出 `...` / `...` 包裹思考，后端解析分离。但依赖模型遵守约定，不可靠。

### 折叠时机（4 轮迭代）

| 方案 | 描述 | 结果 |
|------|------|------|
| v1：首 `content` 后 0.5s 折叠 | 思考区在正式回答开始时收起 | tool calling 多轮导致交替展开/折叠，视觉突兀 |
| v2：立刻折叠、延迟折叠、done 后折叠对比 | 逐一评估 | 全部无法解决多轮交替问题 |
| v3：`done` 时折叠 | 思考区全程展开，`done` 时一次性收起 + 显示回答 | 消除闪烁，牺牲回答流式打字机效果 |
| **v4（最终）**：后端 `thinking_done` 事件在最终轮流末尾驱动折叠 | 后端仅在 `finish_reason == Stop` 时发送 `thinking_done`（流末尾）；前端收到后折叠思考区。`content` 事件同时显示回答气泡并逐字流式吐字 | **多轮安全 + 回答打字机效果兼得** |

> v4 的关键洞察：
> 1. 前端无 reliable 信号判断"思考何时结束"（多轮 tool calling 中 content 可能跨轮才出现，且非最终轮也有 text content）。将边界检测下沉到后端。
> 2. `thinking_seen` 标记追踪 `reasoning_content` 是否出现过，但 `ThinkingDone` 的发送推迟到流末尾。仅在 `finish_reason == Stop`（最终轮、无更多 tool calls）时才触发折叠。
> 3. 非最终轮中 DeepSeek 输出的中间 text（如"我来查一下数据库"）不会触发折叠，思考区在多轮间持续展开。

### 回答气泡显示时机

初版在首 `content` 时显示气泡，导致多轮 tool calling 中空框闪现。v4 中 `content` 事件同时显示气泡 + 流式吐字（无论是否最终轮）。中间轮次的中间 text（如"我来查一下数据库"）短暂可见于气泡中，下一轮思考出现时前端新建消息覆盖。`thinking_done` 仅在最终轮流末尾到达用于折叠思考区，时机在回答流式完成之后。`error`/`catch` 时也显示气泡确保错误可见。

### 思考区 DOM 定位

- 初版：flex row 平级插入（左边缘对齐头像，而非气泡），修正为 `margin-left: 52px` 对齐回答气泡
- 隐藏气泡时使用 `aiDiv.parentElement.style.display = 'none'` 导致整个 flex row（含 AI 头像）消失，改为 `aiDiv.style.display = 'none'` 仅隐藏白色气泡，头像在思考期间保持可见

## 后果

- `reqwest` 成为新依赖
- `stream_and_merge()` 从 ~50 行 typed 流式逻辑变为 ~140 行手动 SSE 解析（含 `thinking_seen` 状态追踪 + `ThinkingDone` 发送）
- 失去 `async-openai` 的 typed error 处理、自动重试等便利，需自行处理 HTTP 错误和 JSON 解析异常
- 后端新增 `Thinking` 和 `ThinkingDone` 事件类型，SSE 协议共 5 种事件
- 前端分三路处理 `thinking`/`thinking_done`/`content`，折叠时机由后端精确控制
- 打字机效果恢复，思考区折叠与回答流式吐字兼得
- 后续若 `async-openai` 加入 `reasoning_content` 支持，可考虑回迁 typed streaming

---

## 第三轮迭代：前端渲染对齐 pi agent（2026-06-12）

### 背景

第二轮迭代后，思考/回答分离在后端和基础前端已跑通。但存在两个问题：

1. **思考区 DOM 位置错误**：思考区通过 `insertBefore` 动态插入在 AI 消息的 flex row 外部，导致出现在回答气泡**下方**而非上方
2. **前端测试页混乱**：存在 `test.html`（简单版）和 `test-tailwind.html`（Tailwind 版）两个版本，后者从未处理 `thinking`/`thinking_done` 事件

统一以 `test-tailwind.html` 为唯一开发测试页，`test.html` 作为简化备选。

### 参考对象：pi agent 渲染模式

分析 pi coding agent（`@earendil-works/pi-agent-core`）的流式渲染架构：

| 层级 | 职责 |
|---|---|
| Provider（`openai-completions.js`） | 忠实地将 thinking / text / toolCall 作为独立 content block 发射，不做任何缓冲或抑制 |
| Agent Loop（`agent-loop.js`） | 透明转发 `message_update` 事件，包含 `assistantMessageEvent` 原样内容 |
| TUI 渲染层 | 按内容块类型分别渲染：thinking 区（灰底小字可折叠）、text 区（Markdown 气泡）、tool 区（状态指示器） |

核心原则：**所有内容块平等对待、独立渲染、不做抑制**。

### 对比：OpenAI Codex Responses API

Codex 使用的是新版 Responses API（非 Chat Completions），其流式事件**按阶段天然隔离**：

```
response.output_item.added  →  item.type="reasoning"     ← 思考阶段
  response.reasoning_text.delta  →  "让我分析..."
response.output_item.done

response.output_item.added  →  item.type="function_call"  ← 工具调用阶段
  response.function_call_arguments.delta
response.output_item.done

response.output_item.added  →  item.type="message"        ← 正式回答阶段
  response.output_text.delta  →  "根据数据..."
response.output_item.done
```

API 层面保证 reasoning → function_call → message 三阶段不重叠。DeepSeek 的 Chat Completions API 使用扁平 delta 结构（`reasoning_content` 与 `content` 同级并列字段），无此能力——这解释了为何 Codex 不需要"中间 content 抑制"而我们面临此问题。

### 决定

#### DOM 结构：列容器布局

```
flex-row
  ├── 头像 AI
  └── msg-col（ml-3 flex-1 列容器）
        ├── thinking-box（💭 思考过程）  ← 上，display:none 初始
        └── answer-bubble（正式回答）    ← 下，display:none 初始
```

思考和回答在同一个列容器内，自然上下排列，不再需要 `insertBefore` hack。

#### 多轮 tool-calling 中间 content 处理

DeepSeek 在中间轮（`finish_reason == ToolCalls`）除了 `reasoning_content`，也会输出 `content`——如"我来查一下数据库"之类的过渡文本。

当前策略：**后端不做抑制**（与 pi agent 一致——忠实地流式输出所有内容）。中间轮 content 会短暂出现在回答气泡中，下一轮工具调用开始后前端会重建 AI 消息（新 `send()` 调用），自然覆盖。若后续需要纯净化体验，可改为后端缓冲策略（中间轮不发送 Content，最终轮 flush）。

#### 侧边栏：历史对话

侧边栏从"快捷提问"改为"历史对话列表"，支持新建/切换/删除。对话存储在浏览器内存中（无后端持久化），首次提问自动创建对话，标题取用户问题前 20 字。

#### 快捷提问位置

4 个快捷提问按钮（工程量 / 进度节点 / 成本分析 / 模板脚手架）移至输入框下方，pill 形状，点击直接发送。

### 已知局限

1. **中间轮 content 闪现**：多轮 tool-calling 中，"我来查一下数据库"等过渡文本会在回答气泡短暂可见。消除需后端改造（中间轮抑制 content）
2. **对话无持久化**：历史对话仅存于浏览器内存，刷新丢失
3. **无 tool call 状态指示**：当前 SSE 协议无 tool call 事件，前端无法展示"正在查询数据..."等中间状态
4. **test.html 未同步**：`test.html` 作为简化备选保留，不含思考区分离功能
