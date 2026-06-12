# 设计拷问记录

> 2026-06-10 · 建造信息 AI 分析系统 · grilled by pi

---

## Q1 · 核心实体

**问：** "建造信息"、"建造空间"、"进度节点"——这些领域的核心实体是什么？MongoDB 里存的到底是什么东西？

**答：** 一个模型。建造信息是属性。

## Q2 · 模型粒度

**问：** "模型"代表什么范围的建筑对象？一个"万达广场1号楼"是一个模型还是多个？

**答：** RVT/IFC 文件解析入库的数据单元。万达广场1号楼可能是一个或多个模型。前端传入指定。

## Q3 · 进度节点来源

**问：** 进度节点是类似"基础施工→主体结构→竣工"这种阶段，还是更细粒度？从哪查？

**答：** 是模型属性里面查询出来的。

## Q4 · 进度节点值格式

**问：** 典型进度节点值是什么样？是 `"基础施工"` 还是数字编号？人能不能看懂？

**答：** 是的，人可读的语义化名称。

## Q5 · 项目架构 & AI 模型

**问：** 准备项目架构、对外 API，接入什么 AI 模型？

**答：** DeepSeek。

## Q6 · 项目结构

**问：** 单体 crate、Cargo workspace、还是先单体后拆？要考虑后续接入更多维度和模型类型。

**答：** C — 先单体，后面再拆。还要考虑如何接入 MongoDB、获取数据。

## Q7 · Web 框架

**问：** Axum、Actix-web、还是 Poem？推荐 Axum（与 tokio 生态原生兼容）。

**答：** Axum。

## Q8 · MongoDB 数据访问层

**问：** 怎么组织数据访问层？A) 直接裸调 Collection，B) Repository trait，C) 轻量查询抽象。推荐 B。

**答：** 有 MCP 或其他模式吗？不想直接去读数据库。

## Q9 · MCP 解耦层级

**问：** 三个方案：A) 独立 MCP Server（可复用），B) 本地 Repository 抽象（轻量），C) MCP 嵌入进程。你是想走正式 MCP 协议，还是"别直接调 MongoDB"就行？

**答：** 还有其他开源的 MCP 工具方式可以读取。

## Q10 · MCP Server 选型

**问：** 现有开源 MongoDB MCP Server：`mongodb-mcp-server`（Node.js）和 `mcp-mongodb`（Python）。但你的模型属性不语义化——字段名可能是拼音/缩写，AI 直接读原始数据容易幻觉。所以需要写自己领域的 MCP Server。

**答：** 使用开源的 MCP，AI 写查询语句然后调用。

## Q11 · 数据可读性

**问：** MongoDB 字段名是语义清晰的还是拼音缩写？这决定要不要加 schema 描述层给 AI。

**答：** 要加一层 schema 描述给 AI。

## Q12 · Schema 注入位置

**问：** Schema 放哪里？A) MCP Tool description，B) 独立 Schema Tool，C) System Prompt。推荐 B（动态可查，不改代码）。

**答：** B。

## Q13 · MCP 集成方式

**问：** MCP Server 与 Axum 怎么通信？A) HTTP/SSE 远程，B) stdio 子进程，C) 内嵌内存 JSON-RPC。

**答：** B 和 C 的优缺点是什么？

## Q14 · stdio vs 内嵌

**问：** B（stdio 子进程）和 C（内嵌）的优缺点对比。

- **B 优点：** 进程隔离、可复用、语言无关 / **B 缺点：** 多一个进程管理、IPC 开销、两个二进制
- **C 优点：** 零 IPC、单二进制、部署简单 / **C 缺点：** 耦合崩溃、不可复用、只能 Rust

**答：** 方案 B。

## Q15 · MCP Server 具体选型

**问：** 选 `mongodb-mcp-server`（Node.js）还是 `mcp-mongodb`（Python）？

**答：** `mongodb-mcp-server`，选这个。

## Q16 · 制式化回答维度

**问：** 5 个固定分析维度——空间位置+进度、成本、工程量、模板脚手架周转——是直接查字段还是需要计算？快捷提问按钮具体交互是什么？

**答：** 让 AI 分析然后生成具体的查询语句。

## Q17 · 用户交互模式

**问：** 用户是直接打字还是点固定按钮？AI 必须固定章节输出还是自由组织但覆盖 5 维度？快捷提问是追问建议还是初始入口？

**答：** 输入模型 ID 然后回去查询信息的。AI 自由组织语言但覆盖这 5 个方面。

## Q18 · 快捷提问按钮

**问：** 快捷提问是 A) 回答下方追问建议，B) 输入框旁固定入口，还是 C) 初始页面预设按钮？

**答：** A。

## Q19 · DeepSeek API 接入

**问：** Rust 调 DeepSeek：A) reqwest 裸调，B) async-openai crate，C) rig/langchain-rust。推荐 B（原生支持 function calling + streaming）。

**答：** B。

## Q20 · MCP Client 实现

**问：** MCP client 库：A) `mcp-client` crate，B) 手写。推荐手写（控制更强，代码量小）。

**答：** 手写控制。

## Q21 · 对话状态管理

**问：** 多轮对话怎么管理？A) 无状态（前端带历史），B) 服务端 session，C) 前端托管。追问时要不要重查数据库还是复用缓存？

**答：** A。先做一版简单的，只记录当前的。

## Q22 · Schema Tool 策略

**问：** 每次请求都调 `describe_model_schema`，还是 system prompt 预埋，还是按模型 ID 返回？

**答：** 每次请求都去调用。

## Q23 · describe_model_schema 实现位置

**问：** 放在 A) 利用 MCP Server 自带 getCollectionSchema，B) Axum 侧做语义包装，C) Fork MCP Server。推荐 B（语义层在 Axum，MCP Server 保持通用）。

**答：** B。

## Q24 · 对外 API 设计

**问：** 要不要流式？追问谁生成？POST 路径？

**答：** 需要支持流式。前端硬编码。统一就一个路径。

## Q25 · SSE 响应结构

**问：** Axum 直接透传 DeepSeek 原始 SSE，还是做统一格式？

**答：** Axum 做一个统一格式给前端。

## Q26 · SSE 事件类型

**问：** 统一格式：`status`（过渡状态）、`content`（文本片段）、`followups`（追问列表）、`done`（结束）、`error`（错误）。要加别的吗？

**答：** 不需要加别的。

## Q27 · 语义标注映射位置

**问：** 放 A) Rust 代码 HashMap，B) YAML 配置文件，C) MongoDB 元数据集合？

**答：** 放一个 YAML/TOML 配置文件，启动时加载。

## Q28 · Crate 模块结构

**问：** 模块拆分：`config` / `api/chat` / `ai/deepseek` / `mcp/client` / `schema`。要增减吗？

**答：** 可以。不需要增减模块。

## Q29 · System Prompt 设定

**问：** 角色定位、5 维度是否必须全量覆盖、输出风格、无数据时怎么办？

**答：** 偏数据报告（表格、数字为主）。无数据跳过该维度。

## Q30 · System Prompt（续）

**问：** 角色定位是什么？5 个维度必须全部覆盖还是按需？

**答：** 分析模型的助手。看用户问什么就答什么维度。

## Q31 · 配置文件内容

**问：** YAML 配置除了 DeepSeek API key、MCP 命令、schema 映射，还要什么？

**答：** 日志和端口号。

## Q32 · 错误处理

**问：** 4 个故障点（Axum、DeepSeek、MCP Server、MongoDB）的错误策略？

**答：** 展示友好错误给用户。

## Q33 · 部署方式

**问：** 开发期 Docker Compose、单机裸跑、还是先不管？

**答：** 单机裸跑。`cargo run` Axum，另开终端 `npx` 跑 MCP Server。

**追问（发现矛盾）：** 前面选了 stdio 子进程通信，意味着 Axum 自己 spawn MCP Server，一行 `cargo run` 全起来。如果"另开终端"得用 HTTP transport。走哪个？

**答：** B（`cargo run` 启动全部）。

## Q34 · MongoDB 数据结构

**问：** 集合名、模型 ID 字段、5 维度字段映射？

**答：** 实际数据展示——`xEntity` 集合。`guid` 是模型 ID。属性在 `propertySet` 数组，`paramGroupId` 分组。展示了 `工程量信息` 分组的文档。后续扩展了 `xLevel`（楼层）和 `xBuilding`（建筑）集合支持关联查询。

**追问：** 进度节点、成本、模板脚手架这三个维度的 `paramGroupId` 叫什么？

**答：** 先实现工程量，其他先不管。

## Q35 · 连接信息

**问：** 集合名、连接字符串格式？

**答：** 集合名：`xEntity`。查询逻辑：前端传 guid → 查文档 → 筛 `propertySet` 中 `paramGroupId == "工程量信息"`。正确。

---

## 决策汇总

| # | 决策点 | 结果 |
|---|---|---|
| Q6 | 项目结构 | 单体 crate，模块拆分，后续拆 workspace |
| Q7 | Web 框架 | Axum |
| Q9 | MCP 解耦 | 开源 MCP Server，AI 写查询语句 |
| Q12 | Schema 注入 | 独立 `describe_model_schema` Tool |
| Q14 | MCP 通信 | stdio 子进程 |
| Q15 | MCP Server | `mongodb-mcp-server`（Node.js） |
| Q18 | 追问按钮 | 回答下方追问建议（前端硬编码） |
| Q19 | DeepSeek SDK | `async-openai` crate |
| Q20 | MCP Client | 手写 JSON-RPC |
| Q21 | 对话状态 | 无状态，前端带完整历史 |
| Q22 | Schema Tool 频率 | 每次请求都调 |
| Q23 | Schema 实现 | Axum 层语义包装 |
| Q24 | 流式 | SSE 流式 |
| Q25 | SSE 格式 | Axum 统一格式（不透明传） |
| Q26 | SSE 事件 | `content` / `done` / `error` |
| Q27 | 语义标注 | YAML 配置启动加载 |
| Q28 | 模块结构 | `config` / `api` / `ai` / `mcp` |
| Q29-Q30 | AI 行为 | 数据报告风格 · 按需覆盖 · 缺数据跳过 |
| Q31 | 配置项 | 日志 + 端口 + API key + MCP + schema |
| Q32 | 错误处理 | 友好错误透传前端 |
| Q33 | 部署 | `cargo run` 启动全部（自动 spawn MCP 子进程） |
| Q34-Q35 | 数据 | 集合 `xEntity` · ID=`guid` · 维度=`propertySet.paramGroupId` |

## 输出文档

| 文档 | 说明 |
|---|---|
| `CONTEXT.md` | 领域术语表 |
| `docs/PRD-建造信息AI分析系统.md` | 产品需求文档 |
| `docs/adr/0001-tech-stack.md` | 技术栈选型（Rust + Axum + async-openai） |
| `docs/adr/0002-mcp-integration.md` | MCP 集成模式（stdio + mongodb-mcp-server） |
| `docs/adr/0003-agent-architecture.md` | AI Agent 架构（Tool Calling + 无状态） |
| `docs/adr/0004-semantic-schema.md` | 语义标注方案（YAML 配置） |
| `docs/adr/0005-sse-api-spec.md` | SSE 接口规范（统一格式 + 流式） |

---

## 第二轮拷问：思考过程分离（2026-06-12）

> 目标：test-tailwind.html 中模型思考过程和正式回答分开，正式回答时思考区域隐藏

### Q1 · 后端协议
**问：** 新增 SSE 事件类型还是 content 内加 subtype？
**答：** 新增 `thinking` 事件类型，与 `content` 平级。

### Q2 · 数据源头
**问：** `deepseek-chat` 开启 thinking 参数，从 `delta.reasoning_content` 拿思考 token？
**答：** 确认。

### Q3 · 前端隐藏机制
**问：** A) `<details>` 折叠 / B) 移除 DOM / C) 渐变淡出保留小字？
**答：** A，可折叠回看。

### Q4 · 折叠触发时机
**问：** A) 立刻 / B) 延迟 0.5s / C) done 后？
**答：** B，0.5 秒延迟 + 视觉过渡。

### Q5 · 思考区域视觉风格
**问：** 灰底、小字、纯文本、CSS 动画，同意吗？
**答：** 同意。

### Q6 · async-openai 版本检查
**问：** 升级新版能支持 `reasoning_content` 吗？
**答：** 0.41.0 也没有此字段，且无 `deny_unknown_fields` 导致静默丢弃。

### Q7 · 采集方案
**问：** A) 绕过 async-openai typed 解析用 reqwest 裸调 / B) 放弃 typed 全用 reqwest？
**答：** A。保留 async-openai 构建消息和 tools，仅替换流式层。

### Q8 · 多字段 fallback
**问：** pi 做了 `reasoning_content` / `reasoning` / `reasoning_text` 多字段兼容，我们要不要？
**答：** 只针对 DeepSeek 的 `reasoning_content`。

### Q9 · 确认开始
**问：** 方案有问题吗？
**答：** 开始实现。

### 决策汇总

| # | 决策点 | 结果 |
|---|---|---|
| Q1 | SSE 协议 | 新增 `SseEvent::Thinking` 事件类型 |
| Q2 | 数据源 | DeepSeek `reasoning_content` + `thinking: {type:"enabled"}` |
| Q3 | 前端折叠 | `<details>` 可折叠组件 |
| Q4 | 折叠时机 | 首 `content` 后延迟 0.5s |
| Q5 | 思考区样式 | 灰底 #f3f4f6、12px 灰色、纯文本、CSS transition |
| Q6 | async-openai | 不升级不改类型，改用 reqwest 裸调 |
| Q7 | 流式采集 | reqwest + 手动 SSE/JSON 解析 |
| Q8 | fallback | 仅 `reasoning_content` |

### 输出文档

| 文档 | 说明 |
|---|---|
| `docs/adr/0006-thinking-stream-separation.md` | 思考过程流式分离架构决策 |
| `CONTEXT.md` | 新增「思考过程」术语 |

---

## 第三轮拷问：流式 Markdown 分块渲染（2026-06-12）

> 目标：改善流式回答的 Markdown 渲染美观度，从提示词和前端解析渲染两方面入手

### Q1 · 分块策略
**问：** 用什么策略判断"这是一个完整的块，可以安全渲染了"？
**答：** 状态机追踪块上下文（PARAGRAPH/FENCE/TABLE/LIST）。

### Q2 · 残缺块可见性
**问：** 残缺块要不要给用户看到？A) 纯文本预览 B) 隐藏不出 C) 尝试解析但容错？
**答：** C，marked 尝试解析残缺块。

### Q3 · 容错边界
**问：** 容错的底线是什么？A) 完全信任 marked B) 白名单 C) 后校验？
**答：** C——marked 渲染后检查未闭合 `<pre>`/`<code>`，有则退回纯文本。

### Q4 · 渲染架构
**问：** A) 全量重渲 + diff B) 增量追加 + 游标？
**答：** B——游标追踪已渲染块数量，只 append 新完整块。

### Q5 · 表格和列表缓冲
**问：** 表格和列表的缓冲策略？A) 完整缓冲 B) 头两行够了 C) 每次重建？
**答：** 表格 C（每次重建），列表 B（逐个 `<li>` append）。

### Q6 · 状态机范围
**问：** 状态机加不加 BLOCKQUOTE 状态？
**答：** 不加。引用极少出现，marked 容错尚可。参考 pi agent 的 md 渲染。

### Q7 · 表格 CSS
**问：** A) pi 风格 B) Tailwind prose C) 自定义？
**答：** A——pi 风格：border-collapse + 细边框 + 浅灰表头。

### Q8 · 字体体系
**问：** A) 全等宽 B) 正文无衬线+代码等宽 C) 全系统默认？
**答：** B——施工人员看数据报告，非源码。

### Q9 · 代码高亮
**问：** A) 引入 hljs B) 不加 C) 极简自己写？
**答：** B——业务数据为主，代码块占比 < 5%，不值得 30KB。

### Q10 · 安全配置
**问：** 照抄 pi 的 marked 安全配置（禁 HTML tag / URL 白名单 / C0 清除）？
**答：** 全做。20 行代码，零信任。

### Q11 · DOM 架构
**问：** A) 单容器 B) 双容器（`.md-settled` + `.md-pending`）？
**答：** B——已完成块只 append，残缺块每次 replace。

### Q12 · 模块四精简
**问：** system-prompt.md 模块四（Markdown 格式约束）6 条，哪些该删？
**答：** 仅保留"字段名加反引号"和"数字格式"。前 4 条（禁 `####`、列表 `-`、段落空行、表格对齐）渲染层负责。

### Q13 · 表格溢出
**问：** A) 提示词限制列数 B) CSS `overflow-x: auto` C) JS 智能拆分？
**答：** B——CSS 兜底，不约束 AI 列数。

### Q14 · breaks:true
**问：** marked 的 `breaks: true` 保留还是移除？
**答：** 保留。AI 输出不保证规范空行，安全网必要。

### Q15 · 模板触发机制
**问：** 模板模式（四维度全出）vs 按需自由回答，怎么切换？
**答：** 语义意图识别——用户问"建造信息"触发四维度模板，问具体维度按需回答。

### Q16 · 追问模式
**问：** 追问"成本再展开"时是模板模式还是自由回答？
**答：** C——混合模式。追问命中某维度用子模板，否则自由。

### Q17 · marked 升级
**问：** CDN v1 没有 `marked.use()` API，做不了安全配置。升级到 v17？
**答：** 同意，`cdn.jsdelivr.net/npm/marked@17`。

### Q18 · 模块一重写
**问：** A) 瘦身为参考手册 B) 拆分为子模块？
**答：** A——保留公式和业务规则，语气从命令式改参考式。

### Q19 · 残缺块视觉
**问：** A) 闪烁光标 B) 半透明 C) 无标记？
**答：** A——`.md-pending` 末尾 `▌` 闪烁。

### Q20 · 代码块流式
**问：** A) 空壳+逐行 append B) 等闭合 C) 当正文？
**答：** A——\`\`\`出现即渲染 `<pre><code>` 壳，逐行 textContent 追加。

### Q21 · 表格重建性能
**问：** 表格每次重建，行数多时要做性能保护吗？
**答：** 不需——现代浏览器无压力，且模块二限流数据量。

### 决策汇总

| # | 决策点 | 结果 |
|---|---|---|
| Q1 | 分块策略 | 状态机追踪块上下文 |
| Q2 | 残缺块可见 | marked 尝试解析，容错渲染 |
| Q3 | 容错边界 | 后校验 `<pre>`/`<code>` 未闭合 |
| Q4 | 渲染架构 | 增量追加 + 游标 |
| Q5 | 表/列缓冲 | 表格满行重建，列表逐 `<li>` append |
| Q6 | 状态机范围 | 4 状态：PARAGRAPH/FENCE/TABLE/LIST |
| Q7 | 表格 CSS | pi 风格 border-collapse + 浅灰表头 |
| Q8 | 字体 | 正文无衬线 + 代码等宽 |
| Q9 | 代码高亮 | 不加，纯等宽展示 |
| Q10 | 安全配置 | 禁 HTML tag / URL 白名单 / C0 清除（tokenizer 覆盖因 marked@12 UMD 不兼容暂移除，URL 白名单待恢复） |
| Q11 | DOM 结构 | 双容器 `.md-settled` + `.md-pending` |
| Q12 | 模块四 | 仅保留：字段名反引号 + 数字格式 |
| Q13 | 表格溢出 | CSS `overflow-x: auto` |
| Q14 | breaks:true | 保留 |
| Q15 | 模板触发 | 语义意图识别，"建造信息"→四维度 |
| Q16 | 追问模式 | 混合——命中维度用子模板 |
| Q17 | marked | CDN `marked@12`（`marked@17` UMD build 404），`marked.setOptions()` |
| Q18 | 模块一 | 瘦身为参考手册 |
| Q19 | 残缺块视觉 | 闪烁光标 `▌` |
| Q20 | 代码块流式 | 空壳 + 逐行 append |
| Q21 | 表格性能 | 直接重建，不设阈值 |

### 输出文档

| 文档 | 说明 |
|---|---|
| `docs/adr/0007-streaming-markdown-chunked-render.md` | 流式 Markdown 渲染美化架构决策（CSS体系+提示词精简已交付，分块状态机已实现待激活） |

### 实现结果

- **CSS 体系**：pi 风格表格、字体、标题扁平化、代码块样式全部生效
- **提示词**：模块一改按需参考、模块四从 6 条缩为 2 条
- **marked 升级**：CDN v1 → v12（v17 无 UMD build）
- **增量渲染**：自研状态机废弃 → 改用 `streaming-markdown`（v0.2.15，3KB gzip，vanilla JS）
  - API：`parser_write(p, chunk)` 喂 chunk，`parser_end(p)` 结束
  - 核心策略：只追加新 DOM 元素，从不修改已有元素（ChatGPT 同款）
  - 加载方式：动态 `import()` 避免 ES module 作用域问题
- **安全配置**：`marked.use()` tokenizer 覆盖与 marked@12 UMD 不兼容，暂时移除
