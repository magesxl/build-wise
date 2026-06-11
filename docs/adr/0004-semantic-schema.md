# 语义标注方案：YAML 配置 + 服务端 Schema 包装

决定将 MongoDB 字段→中文语义的映射放在 YAML 配置文件中，`describe_model_schema` 由 Axum 在服务端组装语义标注后返回给 AI。

## 背景

BIM 数据入库后存储在 MongoDB 的 `xEntity` 集合中。字段名可能是拼音缩写、原始 BIM 属性名（如 `pjd`、`attr_12`），AI 无法直接理解。需要一层语义标注让 AI 能写出正确的查询语句。

## 决定

- **标注存储**：YAML 配置文件（`config.yaml`），启动时加载。`schema.fields` 映射字段名→中文含义，`schema.property_groups` 映射 `paramGroupId`→分组含义
- **标注注入**：在 Axum 进程侧做语义包装。AI 调用 `describe_model_schema` 时，Axum 读配置文件动态生成带语义的 schema 描述文本返回
- **MCP Server 职责**：仅负责原始数据查询（`listCollections`、`getCollectionSchema`），不承担语义标注

## 考虑过的方案

- **写死在 Rust 代码**：零依赖，但改映射需要重新编译部署，BIM 字段随项目版本变化时不灵活
- **存在 MongoDB 元数据集合**：`describe_model_schema` 实时查元数据，最灵活但多一跳查询，且语义知识是业务知识不是数据
- **Fork MCP Server 加自定义 Tool**：在 `mongodb-mcp-server` 内实现 `describe_model_schema`，但语义映射是项目特定知识，放通用 Server 里不合适

## 后果

- 修改字段映射只需改 YAML 重启服务，不用重新编译
- 语义标注逻辑集中在 Axum 侧，MCP Server 保持通用
- `describe_model_schema` 返回的文本由配置驱动，内容可精确控制
- YAML 适合人工维护（中文注释、描述），相比 TOML 在嵌套结构上更自然
