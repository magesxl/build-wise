# 流式 Markdown 渲染美化：CSS 体系 + streaming-markdown

决定废弃自研分块渲染状态机，改用 `streaming-markdown` 开源库（vanilla JS，3KB gzip）实现增量 DOM 渲染。保留 CSS 体系重构和提示词精简作为独立交付。

## 背景

当前前端在每个 SSE chunk 到达时对累积全文执行 `marked.parse(fullText)` + `innerHTML` 全量替换。流式 Markdown 渲染天然不美观——半截表格、未闭合列表、闪烁 DOM。

## 决定

### 增量渲染：streaming-markdown

选用 **`streaming-markdown`**（v0.2.15）替代自研状态机。核心优势：

- **"只追加新 DOM，不修改已有元素"**——ChatGPT 同款策略，天然消除闪烁
- **3KB gzip**，零依赖，vanilla JS
- **CDN 可用**：`jsdelivr` 直接引用，动态 `import()` 免模块作用域问题
- 支持表格、嵌套列表、代码块、blockquote、任务列表、LaTeX 等完整 Markdown 语法
- API 极简：`parser_write(p, chunk)` → 喂 chunk；`parser_end(p)` → 结束

### CSS 体系（对齐 pi agent）

- 表格：`border-collapse` + 细边框 + 浅灰表头 + 斑马条纹 + `overflow-x: auto`
- 字体：正文系统无衬线 + 代码等宽
- 标题扁平化：所有标题 `1em`
- 代码块：灰底圆角，无语法高亮
- 双容器结构：`.md-output`（parser 渲染目标）+ `.md-cursor`（闪烁光标）

### 提示词（独立交付）

- 模块一：命令式模板 → 按需参考框架
- 模块四：6 条 → 2 条（仅字段名反引号 + 数字格式），释放约 500 token

## 考虑过的方案

| 方案 | 结果 |
|------|------|
| 自研状态机 `splitBlocks` | ❌ cursor 追踪 bug（21→20 跳变），在 DeepSeek chunk 粒度下不稳定 |
| `remend` | ❌ 只解决残缺块容错，不解决 DOM 管理 |
| `stream-markdown-parser` | ❌ 需替换 marked → markdown-it，改动大 |
| `@deltakit/markdown` | ❌ React 专属 |
| `streaming-markdown` | ✅ 增量 DOM + 3KB + vanilla JS + ChatGPT 策略 |

## 踩坑记录

- **marked@17**：无 UMD build，CDN 返回 404 → 改用 marked@12
- **`marked.use()` tokenizer 覆盖**：与 marked@12 UMD 不兼容，破坏全部 `marked.parse()` 输出 → 移除
- **自研 cursor 追踪**：DeepSeek SSE chunk 粒度导致连续事件间 blocks.length 非单调（21 → 20），cursor 超界 → 废弃
- **ES module 导入**：`streaming-markdown` 是 ES module → 使用动态 `import()` 避免 `<script type="module">` 带来的 onclick 全局作用域问题

## 后果

- 代码量：删除自研状态机 ~60 行 → 新增 3 行 `parser_write` / `parser_end` 调用
- 新增依赖：`streaming-markdown`（3KB CDN），`marked@12`（保留用于历史消息回放）
- CSS 从 Tailwind reset 裸奔 → pi 风格完整体系
- 提示词模块四释放约 500 token
- 代码块无语法高亮（不引入 hljs）
