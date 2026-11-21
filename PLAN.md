# Typecho-rs 规划

> 目标：用 Rust 重写一个轻量、可靠、可长期维护的个人博客 + 笔记系统，并原生支持 MCP，让 AI 可以参与写作、整理和 review。
>
> 参考实现：当前目录下的 Typecho 源码（`install/*.sql`、`var/Widget/*`、`install.php`）。

## 当前实现状态

已落地（可运行 MVP）：

- [x] SQLite schema + migrations + seed
- [x] 管理员登录、Session、CSRF、限流、改密
- [x] 文章/页面 CRUD、草稿/发布/回收站
- [x] 分类/标签、搜索、日期归档、分页
- [x] 评论提交、审核、嵌套展示
- [x] Markdown、代码高亮、HTML 过滤、Wiki 链接
- [x] RSS/Atom/sitemap/robots/Open Graph
- [x] 笔记工作台、反向链接、转文章/同步
- [x] 修订历史
- [x] 媒体上传
- [x] MCP stdio + HTTP、Token/scope、review 工具
- [x] Dockerfile / docker-compose / README
- [x] 单元测试 + 集成 smoke test
- [x] devorz / Cactus 文章导入脚本（正文、图片、分类、标签、日期）
- [x] 后台分栏实时预览 + Markdown 工具栏
- [x] Mermaid / UML 图表（flowchart、sequence、class、state、ER 等）
- [x] 扩展代码高亮语法集（Kotlin、Swift、Dart 等）
- [x] 前后台 shadcn/ui 风格（CSS token 移植，无 React 依赖）
- [x] light / dark / system 主题切换（localStorage 记忆）

后续可选：

- [ ] Typecho 数据导入
- [ ] S3/对象存储
- [ ] FTS5 trigram 全文搜索
- [ ] 更细的后台 review 界面（revision diff）
- [ ] 多用户/角色
- [ ] 封面图/OG image

## 已确认决策（2026-10-31）

| 项 | 决定 |
|---|---|
| 数据库 | 现在用 SQLite；领域层/仓储层不绑定 SQLite，后续可加 PostgreSQL |
| 笔记可见性 | 仅后台私有；转成博客文章后才公开 |
| AI 默认权限 | `read` + `write` + `review`；默认不能发布 |
| 旧数据迁移 | 暂不做，后续作为可选命令 |
| 评论 | 自带评论 + 审核 |
| 界面 | SSR + 少量原生 JS |
| 部署 | 提供 Dockerfile + docker-compose |
| 语言/时区 | 中文，默认 `Asia/Shanghai`（UTC+8） |
| 媒体 | 本地磁盘；用 Storage trait 隔离，后续可接 S3 |
| 公开站点增强 | sitemap、Open Graph、服务端代码高亮 |

---

## 1. 范围与边界

### 保留的博客基础能力

- 文章（post）与独立页面（page）
- 分类、标签
- Markdown 写作与渲染
- 草稿、发布、回收站
- 评论与评论审核
- 首页、文章页、分类/标签/日期归档、搜索、分页
- RSS / Atom
- 后台登录、内容管理、设置、媒体上传
- 简单的文章修订历史

### 明确不做

- Typecho 主题兼容、模板引擎兼容
- 插件系统、Hook 系统
- XML-RPC
- 多站点、多租户
- 复杂角色权限（先做单管理员/少量编辑者）
- 像素级复刻 Typecho 后台

### 新增能力

- Notes 笔记工作台：快速记录、标签、状态流转
- 笔记一键转为博客文章，并保留关联
- Wiki 链接 `[[笔记标题]]` 与反向链接
- MCP Server：stdio + HTTP，两种传输
- AI 写作 / 整理 / review 工作流
- 内容修订与操作审计，方便 review 和回滚

---

## 2. 技术选型

| 层 | 选择 | 理由 |
|---|---|---|
| 语言 | Rust 2024 edition | 当前工具链已支持 |
| Web | axum 0.8 + tokio | 轻量、生态成熟、易做 MCP HTTP |
| 数据库 | SQLite + sqlx（后续可加 PostgreSQL） | 单作者博客足够；单文件备份；WAL 并发可接受；仓储层隔离 SQL 方言 |
| 模板 | Askama | 编译期检查、服务端渲染、无前端构建链 |
| Markdown | pulldown-cmark | 纯 Rust、可控、可扩展 |
| 密码 | argon2 | 现代密码哈希 |
| 日志 | tracing + tracing-subscriber | 结构化日志 |
| 前端 | 服务端 HTML + 少量原生 JS | 不做 SPA，降低维护成本 |

### 部署形态

- 单个二进制 + 一个 `data/` 目录
  - `data/typecho.db`
  - `data/uploads/`
- 默认监听 `127.0.0.1:3000`，由 Caddy/Nginx 反代
- 提供 Dockerfile / docker-compose（`data/` 挂载为 volume）
- SQLite 使用 WAL、foreign_keys、busy_timeout
- 媒体走 `Storage` trait：当前 `LocalStorage`，未来加 `S3Storage` 不影响业务层
- 备份 = 复制 `data/`（提供 `inkwell backup` 命令更好）

> 如果未来需要多用户/高并发，可把 sqlx 切换到 PostgreSQL。为此：
> - 业务代码只依赖仓储接口，不直接拼 SQL
> - 不把 SQLite 特有语法散落到业务层
> - 搜索先用 `LIKE` 兜底，FTS5 作为 SQLite 的可选优化
> - 迁移文件按方言拆分，后续增加 Postgres migration

---

## 3. 领域模型

### 3.1 核心表

```sql
users (
  id, username, password_hash, display_name, email, role,
  created_at
)

contents (
  id,
  kind,              -- post | page | note
  title, slug, body, excerpt,
  status,            -- 见下方状态机
  visibility,        -- public | private
  author_id,
  created_at, updated_at, published_at,
  review_status,     -- none | pending | approved | changes_requested
  review_notes,
  allow_comments,
  source_note_id,    -- 由哪篇笔记转来
  promoted_to_id,    -- 笔记转成了哪篇文章
  metadata,          -- JSON，保留未来扩展
  deleted_at
)

terms (
  id, kind,          -- category | tag
  name, slug, description,
  parent_id,         -- 分类层级预留
  created_at
)

content_terms (content_id, term_id)

comments (
  id, content_id, parent_id,
  author, email, url, body,
  status,            -- pending | approved | spam | trash
  ip, user_agent, created_at
)

revisions (
  id, content_id, title, body, status,
  actor,             -- human:<uid> | mcp:<token-name> | system
  note, created_at
)

links (
  source_id, target_title, target_id,
  PRIMARY KEY (source_id, target_title)
)

media (
  id, filename, original_name, mime, size,
  width, height, created_at
)

sessions (
  token_hash, user_id, csrf_token,
  created_at, expires_at, ip, user_agent
)

api_tokens (
  id, name, token_hash, scopes,
  created_at, last_used_at, revoked_at
)

settings (key, value)
```

### 3.2 状态机

文章/页面：

```text
draft ──publish──> published ──trash──> trash
  ↑                    │                 │
  └────unpublish───────┘                 └──restore──> draft
```

笔记：

```text
inbox ──> developing ──> evergreen ──> archived
  └────────────── promote ──────────────┘
```

- `review_status` 独立于发布状态：
  - `none`：未进入 review
  - `pending`：等待 review
  - `approved`：review 通过
  - `changes_requested`：需要修改
- 笔记默认且当前只作为后台私有内容：不进入公开路由、RSS、搜索和 sitemap。
- `visibility=private` 的文章/页面也只允许后台/MCP 读取，不进入公开查询。

### 3.3 笔记转博客

原则：**笔记不销毁，博客文章是它的“发布视图”**。

- 笔记上保存 `promoted_to_id`
- 文章上保存 `source_note_id`
- 转换时复制：标题、正文、标签、摘要
- 默认创建 `draft`，由人或具备 `publish` 权限的 AI 发布
- 再次执行 promote 时：
  - 已有文章则返回已有文章
  - 可选 `sync=true` 把笔记最新内容同步过去
- 文章与笔记保持可追溯关系，不自动双向同步，避免“笔记改一个字博客就变”的意外

### 3.4 Wiki 链接与反向链接

- 语法：`[[笔记标题]]` 或 `[[slug]]`
- 保存时解析并写入 `links`
- 笔记详情显示：
  - 出链
  - 反向链接（哪些笔记引用了它）
  - 是否已转成博客
- 渲染策略：
  - 后台笔记：链接到 `/admin/notes/{id}`
  - 公开文章：若目标笔记已 promote，则链接到对应文章；否则显示纯文本

---

## 4. 路由设计

### 4.1 公开站点

```text
GET  /                         首页（最新文章）
GET  /archives/{slug}          文章详情
GET  /pages/{slug}             独立页面
GET  /categories/{slug}        分类归档
GET  /tags/{slug}              标签归档
GET  /archive/{year}/{month}   日期归档
GET  /search?q=                搜索
GET  /feed.xml                 RSS 2.0
GET  /atom.xml                 Atom
GET  /media/{path}             上传文件
GET  /static/*                 静态资源
POST /comments/{content_id}    发表评论
```

### 4.2 后台

```text
GET/POST /admin/login
POST     /admin/logout
GET      /admin                         仪表盘 + review 队列
GET      /admin/{kind}                  列表：posts/pages/notes
GET/POST /admin/{kind}/new
GET/POST /admin/{kind}/{id}/edit
POST     /admin/{kind}/{id}/delete
POST     /admin/{kind}/{id}/restore
POST     /admin/{kind}/{id}/publish
POST     /admin/notes/{id}/promote
GET/POST /admin/review
GET      /admin/comments
POST     /admin/comments/{id}/status
GET/POST /admin/terms
GET/POST /admin/settings
GET/POST /admin/media
GET/POST /admin/tokens
POST     /admin/preview                 Markdown 预览
```

### 4.3 MCP

```text
POST /mcp                    Streamable HTTP（Bearer Token）
inkwell mcp --stdio       stdio 模式（本地 AI 客户端）
```

---

## 5. MCP 设计

### 5.1 协议

- JSON-RPC 2.0
- 初始支持 MCP `2024-11-05`，结构上兼容后续版本
- 支持：
  - `initialize`
  - `notifications/initialized`
  - `ping`
  - `tools/list`
  - `tools/call`
  - `resources/list`
  - `resources/read`
- HTTP 使用 `POST /mcp`，返回 `application/json`
- stdio 使用换行分隔 JSON（stdout 只输出协议消息，日志走 stderr）

### 5.2 认证与权限

- HTTP：`Authorization: Bearer <token>`
- Token 存哈希，不存明文；后台可创建/吊销
- 环境变量 `MCP_TOKEN` 可作为部署级 token
- Scope：
  - `read`
  - `write`
  - `publish`
  - `review`
  - `admin`
- 默认 AI token 建议：`read,write,review`，**不默认给 publish**
- stdio 模式继承本机权限，不做额外认证

### 5.3 工具清单

内容：

- `content_list`
- `content_get`
- `content_create`
- `content_update`
- `content_delete`
- `content_publish`
- `content_search`
- `content_stats`

笔记：

- `note_list`
- `note_get`
- `note_create`
- `note_update`
- `note_promote`

Review：

- `review_queue`
- `review_submit`
- `review_decide`
- `revision_list`
- `revision_get`

评论：

- `comment_list`
- `comment_moderate`

### 5.4 工具行为约定

- `content_create/update` 默认只写草稿，不自动发布
- 所有写操作创建 revision，记录 actor
- `content_update` 支持 `expected_updated_at` 乐观锁，避免 AI 覆盖人工修改
- `review_decide`：
  - `approved` / `changes_requested`
  - 可带 `publish=true`，但要求 token 有 `publish` scope
- 返回结果同时包含：
  - `content: [{ type: "text", text: ... }]`，方便所有客户端
  - `structuredContent`，方便现代客户端直接解析
- 错误通过 `isError: true` 返回，不抛协议级异常

### 5.5 Resources

- `typecho://content/{id}`
- `typecho://notes/{id}`
- `typecho://review/queue`

用于让支持 resources 的客户端直接浏览待 review 内容。

---

## 6. AI Review 工作流

### 6.1 人写，AI review

```text
人创建/修改草稿
  -> review_submit(id)            标记 pending
  -> AI review_queue               取待审
  -> AI content_get + revision_list
  -> AI review_decide(approved / changes_requested, notes)
  -> 人查看 review_notes 后决定发布
```

### 6.2 AI 写，人 review

```text
AI content_create / note_create   写草稿
  -> 自动 review_status=pending（可配置）
  -> 人在后台 review 队列查看
  -> 人批准/修改/发布
```

### 6.3 安全默认值

- AI 默认不能直接发布
- AI 默认只能把内容移到回收站；永久删除需要 `admin` scope
- 修改/删除已发布内容需要 `publish` scope
- 所有 AI 操作有 actor 记录
- 重要操作（publish/delete/token）要求显式 scope

---

## 7. 安全设计

- 密码：argon2id
- 会话：随机 token，数据库存 SHA-256，HttpOnly + SameSite=Lax + Secure（HTTPS 时）
- CSRF：后台所有写操作校验 session 内 CSRF token
- 登录失败限流（简单内存计数 + 退避）
- Markdown：禁用原始 HTML，或过滤 HTML 事件，防止 XSS
- 评论：纯文本转义 + 换行处理；默认审核
- 上传：限制大小/类型，随机文件名，禁止路径穿越
- MCP token：哈希存储、scope、可吊销、记录 last_used_at
- 响应头：CSP、X-Content-Type-Options、Referrer-Policy、X-Frame-Options
- 审计：revisions + actor
- 备份：文档说明 `data/` 备份；后续可加 `backup` 子命令

---

## 8. 迭代计划

### M0：确认与骨架

- 确认本文档中的待定项
- 建立 Cargo 项目结构、配置、错误类型
- SQLite migration、连接池、种子管理员
- 验收：`cargo run` 能启动并完成建库

### M1：基础设施与认证

- settings、users、sessions
- 登录/登出、CSRF、后台布局
- Markdown 渲染、slug 生成、摘要
- 验收：能登录后台，看到空仪表盘

### M2：公开博客

- 文章/页面 CRUD
- 分类/标签
- 首页、详情、归档、搜索、分页
- 评论提交与展示
- RSS/Atom
- 验收：一个可公开访问的博客跑通

### M3：后台完善

- 内容列表、筛选、回收站
- 评论审核
- 分类/标签管理
- 设置页
- 媒体上传
- Markdown 预览
- 验收：不碰数据库也能完成日常运营

### M4：Notes 工作台

- 笔记 CRUD、inbox/developing/evergreen 状态
- 标签、Wiki 链接、反向链接
- 笔记转博客、同步
- 修订历史
- 验收：从随手记到发布文章的完整链路

### M5：MCP 与 AI Review

- stdio server
- HTTP `/mcp` + Bearer + scopes
- 内容/笔记/review/评论工具
- review 队列与后台联动
- 验收：Claude/Cursor 等客户端可创建笔记、改草稿、提交 review

### M6：加固与交付

- 单元测试 + 集成测试
- 限流、安全头、错误页
- Dockerfile / systemd / 反代示例
- 备份与恢复文档
- （可选）Typecho 数据导入
- 验收：可长期部署使用

---

## 9. 测试策略

### 单元测试

- slug 生成与去重（中文、空格、符号）
- Markdown 渲染与原始 HTML 过滤
- 摘要与 `<!--more-->`
- Wiki 链接解析
- 状态机与权限判断

### 集成测试

- 登录 / CSRF / 会话过期
- 文章 CRUD / 发布 / 回收站
- 评论提交与审核
- 笔记 promote / sync / 幂等
- MCP initialize / tools/list / tools/call
- MCP scope 拒绝（无 publish 不能发布）
- 乐观锁冲突

### 手工冒烟

- 首次启动、建管理员、写文章、评论、RSS
- AI 客户端通过 HTTP/stdio 完成一轮 review

---

## 10. 风险与取舍

| 风险 | 处理 |
|---|---|
| 范围膨胀 | 严格不做主题/插件；先完成内容闭环 |
| 中文 slug/搜索 | slug 保留 Unicode；搜索用 FTS5 trigram 或 LIKE 兜底 |
| SQLite 并发 | WAL + busy_timeout；单作者场景足够 |
| AI 误发布 | 默认无 publish scope；revision + review |
| MCP 协议演进 | 协议层与业务层分离，工具 schema 独立 |
| 旧数据迁移 | 作为可选 M6 项，确认后再做 |
| Markdown XSS | 禁用原始 HTML，评论纯文本 |
| 备份遗漏 | 文档明确 data/ 备份；后续加命令 |

---

## 11. 待确认项

全部确认：

1. 数据库：SQLite 起步，仓储层预留 PostgreSQL。
2. 笔记：仅后台私有，转成文章后公开。
3. AI：默认读写 + review，不能发布。
4. 旧数据：暂不迁移，后续可选。
5. 评论：自带评论 + 审核。
6. 界面：SSR + 少量原生 JS。
7. 部署：Docker + docker-compose；同时提供本地运行方式。
8. 语言/时区：中文，默认 `Asia/Shanghai`。
9. 媒体：本地磁盘，Storage trait 预留 S3。
10. 公开站点：sitemap、Open Graph、服务端代码高亮。
