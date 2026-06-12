# 流式 Markdown 渲染美化：分块策略 + 语义提升

决定通过 CSS 体系重构 + 提示词精简来改善流式 Markdown 渲染效果。分块渲染状态机已实现但因 cursor 追踪问题暂未启用，当前回退为全量 `marked.parse()` 渲染。

## 背景

当前前端在每个 SSE chunk 到达时对累积全文执行 `marked.parse(fullText)` + `innerHTML` 全量替换。流式 Markdown 渲染天然不美观——半截表格、未闭合列表、闪烁 DOM。

开源方案调研结果：
- **`remend`**（Vercel）：自愈式 Markdown 解析，处理残缺块。零依赖、框架无关
- **`stream-markdown-parser`**：基于 markdown-it-ts 的流式 AST 解析器，框架无关
- **`@deltakit/markdown`**：React 专属组件，"settled blocks memoized"
- 以上三方案均不适用于 vanilla JS 项目——要么框架绑定，要么替换 marked 成本过高

## 决定

**双轨策略**：CSS 体系 + 提示词立即生效；分块渲染状态机作为基础设施预留。

### 已交付部分

1. **marked 升级**：CDN v1 → `marked@12`（v17 UMD build 404，v12 可用且有 `marked.use()` API）
2. **CSS 体系重构**（对齐 pi agent 风格）：
   - 表格：`border-collapse` + 细边框 + 浅灰表头 + 斑马条纹
   - 字体：正文系统无衬线 + 代码等宽
   - 标题扁平化：所有标题 `1em`
   - 代码块：灰底圆角，无语法高亮
   - 表格溢出：`overflow-x: auto`
3. **双容器 DOM**：`.md-settled`（已完成块）+ `.md-pending`（进行中块 + 闪烁光标）
4. **提示词精简**：
   - 模块一：命令式模板 → 按需参考框架
   - 模块四：6 条 → 2 条（仅字段名反引号 + 数字格式）
5. **marked 安全配置**：URL 白名单 link/image renderer（`marked.use()` renderer 覆盖曾因兼容性问题移除，待后续恢复）

### 状态机实现（代码就绪，未激活）

`splitBlocks()` 函数已实现（~60 行），支持 PARAGRAPH / FENCE / TABLE / LIST 四状态追踪。`renderBlock()` + `validateHtml()` 后校验函数已就绪。

当前 content/done handler 使用简化的全量渲染（`marked.parse(fullText)` → `.md-settled` innerHTML），因为 cursor 追踪存在 off-by-one 问题：DeepSeek 流式 chunk 粒度导致 `blocks.length` 在连续事件间跳变（21→20），cursor 超过当前 block 数量后无法正确 append 完成块。

## 考虑过的方案

- **`remend`**：只解决残缺块容错，不解决分块架构和 DOM 管理
- **`stream-markdown-parser`**：需替换 marked 为 markdown-it，改动面大
- **marked@17**：无 UMD build，CDN 引用返回 404
- **`marked.use()` tokenizer 覆盖**：API 格式与 marked@12 UMD 不兼容，会破坏 `marked.parse()` 全部输出

## 后果

- marked CDN 版本：v1 → v12
- CSS 从零（Tailwind reset裸奔）→ pi 风格完整体系（~30 行）
- 提示词模块四释放约 500 token 给业务逻辑
- 分块渲染状态机代码已就绪（`splitBlocks` / `renderBlock` / `validateHtml`），待 cursor bug 修复后启用
- 代码块无语法高亮（不引入 hljs），保持依赖精简
