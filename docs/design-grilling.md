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

**答：** 实际数据展示——`xEntity` 集合。`guid` 是模型 ID。属性在 `propertySet` 数组，`paramGroupId` 分组。展示了 `工程量信息` 分组的文档。

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
