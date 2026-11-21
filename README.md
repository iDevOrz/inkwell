# typecho-rs

一个用 Rust 重写的轻量个人博客 + 笔记系统，原生支持 MCP，方便让 AI 参与写作、整理和 review。

参考了 Typecho 的数据模型和基础功能，但**不兼容 Typecho 主题/插件**，也不追求后台像素级复刻。目标是：单二进制、SQLite、部署简单、长期可维护。

模板和静态资源在编译时嵌入二进制，部署时只需要 `typecho-rs` 可执行文件 + `data/` 目录。

视觉风格：前后台统一 **shadcn/ui 风格**（CSS token 移植，不引入 React/Tailwind 构建链），支持 `light / dark / system` 主题切换。

---

## 功能

- 文章、独立页面、草稿、发布、回收站
- 分类、标签
- Markdown 写作与渲染
- 服务端代码高亮（syntect）
- 评论 + 审核
- 首页、分类/标签/日期归档、搜索、分页
- RSS 2.0 / Atom / sitemap / robots.txt
- Open Graph
- 后台内容管理、评论审核、设置、媒体上传
- 笔记工作台：inbox / developing / evergreen / archived
- Wiki 链接 `[[笔记标题]]` 与反向链接
- 笔记一键转为博客文章，可同步更新
- 修订历史（每次保存记录 actor）
- MCP Server：stdio + HTTP，Bearer Token + scope
- AI review 工作流
- 默认中文、UTC+8

## 不包含

- Typecho 主题/插件兼容
- XML-RPC
- 多站点、多租户
- 复杂角色权限
- 前端 SPA / 构建链
- 文章密码保护（当前只有 public / private）
- 自定义字段与主题模板

---

## 快速开始

### 本地运行

```bash
cd typecho-rs
cp .env.example .env
# 编辑 .env，至少设置 ADMIN_PASSWORD 和 SITE_URL

cargo run -- serve
```

打开 <http://127.0.0.1:3000>，后台在 <http://127.0.0.1:3000/admin>。

首次启动会自动：

- 创建 SQLite 数据库和迁移
- 创建管理员（用户名来自 `ADMIN_USER`，密码来自 `ADMIN_PASSWORD`；未设置则随机生成并打印到日志）
- 创建默认分类和一篇欢迎文章

### Docker

```bash
# 配置环境变量；不要把 .env 提交到 Git
cp .env.example .env
# 至少设置 ADMIN_PASSWORD；公网部署时将 SITE_URL 设为 HTTPS 地址。
# MCP_TOKEN 可留空（之后可在后台创建独立 Token）。
docker compose up -d --build
```

数据保存在 `./data`：

```text
data/
  typecho.db
  typecho.db-wal
  typecho.db-shm
  uploads/
```

备份直接复制整个 `data/` 目录即可。更稳妥的方式是先停止服务，或使用 SQLite 的在线备份：

```bash
sqlite3 data/typecho.db ".backup 'data/backup.db'"
```

> Linux 上使用 bind mount 时，容器内以 UID 10001 运行。如果遇到数据库无法写入，
> 执行 `sudo chown -R 10001:10001 ./data`，或改用 Docker named volume。

### 反向代理

Caddy 示例：

```caddyfile
example.com {
    reverse_proxy 127.0.0.1:3000
}
```

`SITE_URL` 要设置成外部访问地址（`https://example.com`），这样 RSS、sitemap、Open Graph 和 Secure Cookie 才是正确的。

---

## 配置

| 环境变量 | 默认值 | 说明 |
|---|---|---|
| `BIND_ADDR` | `127.0.0.1:3000` | 监听地址 |
| `DATA_DIR` | `./data` | 数据库和上传文件目录 |
| `DATABASE_URL` | `sqlite://<DATA_DIR>/typecho.db` | SQLite 连接串 |
| `SITE_URL` | `http://127.0.0.1:3000` | 站点外部地址 |
| `ADMIN_USER` | `admin` | 首次启动创建的用户名 |
| `ADMIN_PASSWORD` | 随机生成 | 首次启动创建的管理员密码 |
| `MCP_TOKEN` | 空 | HTTP MCP 的部署级 Bearer Token |
| `MCP_TOKEN_SCOPES` | `read,write,review` | 部署级 Token 的权限 |
| `COOKIE_SECURE` | 根据 `SITE_URL` 自动判断 | 是否强制 Secure Cookie |
| `SESSION_DAYS` | `14` | 登录有效期 |
| `MAX_UPLOAD_BYTES` | `10485760` | 单文件上传上限 |
| `RUST_LOG` | `typecho_rs=info,tower_http=info` | 日志级别 |

站点标题、描述、每页文章数、时区、评论设置等在后台 `/admin/settings` 修改。

---

## 主题

前台、后台和登录页都支持三种主题：

- `Light`
- `Dark`
- `System`（跟随操作系统）

主题保存在浏览器 `localStorage`，切换入口在：

- 前台页头
- 后台顶栏
- 登录页右上角

主题切换会同步更新 Vditor 编辑器、Mermaid 图表和代码块周边样式。

## 编辑器

后台视觉采用 **shadcn/ui 风格**（CSS token 移植，不引入 React/Tailwind 构建链），编辑器使用 **Vditor 4.0.1** 的 `sv` 分屏模式：

- 左侧 Vditor 编辑区，右侧实时预览
- Vditor 提供工具栏、快捷键、大纲、全屏、代码块、表格、上传
- 预览通过 `preview.url` 回到 `/admin/vditor/preview`，由 Rust 服务端渲染
- 因此预览和最终文章使用同一套 Markdown、代码高亮、HTML 过滤和 Mermaid 渲染
- 上传走 `/admin/vditor/upload`，直接进入媒体库
- Vditor 的 `dist` 已放在 `static/vditor/`，不依赖 unpkg/CDN
- Vditor 使用 MIT 协议，版本固定在 `4.0.1`

> Vditor 的图标脚本会以内联方式注入，CSP 里对 `ant.js` 做了 SHA-256 白名单。升级 Vditor 时需要同步更新 `src/web/mod.rs` 里的 `script-src` hash。

### Mermaid / UML

正文里使用 fenced code block：

````markdown
```mermaid
classDiagram
  class Note {
    +String title
    +promote()
  }
  class Post {
    +String slug
    +publish()
  }
  Note --> Post : promote
```
````

支持 Mermaid 的 `flowchart`、`sequenceDiagram`、`classDiagram`、`stateDiagram-v2`、`erDiagram`、`gantt`、`journey` 等。Mermaid 使用 `securityLevel: "strict"`，不会执行图表里的 HTML/脚本。

Mermaid 脚本只在需要它的文章页和后台编辑页加载，不会拖慢普通文章。

> Vditor 本身也支持 PlantUML、Graphviz、ECharts、MathJax 等；当前只启用了 Mermaid，其余能力按需再加。

## MCP

### 传输方式

1. **stdio**：适合本地 AI 客户端直接启动进程

```bash
typecho-rs mcp
```

2. **HTTP**：随 Web 服务启动

```text
POST /mcp
Authorization: Bearer <token>
Content-Type: application/json
```

HTTP 使用 `MCP_TOKEN` 环境变量，或在后台 `/admin/tokens` 创建独立 Token。Token 只保存哈希，创建时显示一次。

### 权限

| Scope | 能力 |
|---|---|
| `read` | 读取内容、笔记、评论、统计、修订 |
| `write` | 创建/更新草稿和笔记、提交 review、移到回收站 |
| `review` | 审核内容、审核评论 |
| `publish` | 发布文章/页面、发布笔记转换的文章 |
| `admin` | 全部权限 |

默认 AI Token 建议只给 `read,write,review`。**发布必须人工确认，或显式创建带 `publish` 的 Token。**

### 工具

内容：

- `content_list` / `content_get` / `content_create` / `content_update`
- `content_delete` / `content_publish` / `content_search` / `content_stats`

笔记：

- `note_list` / `note_get` / `note_create` / `note_update` / `note_promote`

Review：

- `review_queue` / `review_submit` / `review_decide`
- `revision_list` / `revision_get`

划词批注：

- `annotation_queue` / `annotation_list` / `annotation_get`
- `annotation_create` / `annotation_reply` / `annotation_resolve`

AI 版本：

- `proposal_create` / `proposal_list` / `proposal_get`
- `proposal_apply` / `proposal_reject`

评论：

- `comment_list` / `comment_moderate`

资源：

- `typecho://review/queue`
- `typecho://annotations/queue`
- `typecho://notes/recent`
- `typecho://content/{id}`

### Claude Desktop 配置（stdio）

```json
{
  "mcpServers": {
    "typecho-rs": {
      "command": "/absolute/path/to/typecho-rs",
      "args": ["mcp"],
      "env": {
        "DATA_DIR": "/absolute/path/to/data",
        "SITE_URL": "https://example.com"
      }
    }
  }
}
```

### HTTP 客户端配置

```json
{
  "mcpServers": {
    "typecho-rs": {
      "url": "http://127.0.0.1:3000/mcp",
      "headers": {
        "Authorization": "Bearer tcr_xxxxxxxxxxxxxxxxx"
      }
    }
  }
}
```

具体字段名以你的 MCP 客户端文档为准。

---

## 笔记到博客的工作流

### 划词批注，AI 改稿，人发布

```text
人在文章/笔记编辑页打开「划词批注」
  -> 选中文字，写批注；或写整体评价
  -> AI 调 annotation_queue 拉取 open 批注
  -> AI 调 content_get 读正文
  -> AI 调 proposal_create 生成同一篇文章的「待审版本」
  -> AI 调 annotation_reply 回复并标记 addressed
  -> 人在文章编辑页的「AI 版本」里看结果预览和 diff
  -> 人选择应用或拒绝；应用后才更新正文并生成 revision
  -> 人自己发布
```

入口：

- 后台「文章」/「笔记」列表每行的「批注」
- 编辑页右上角或底部操作区的「划词批注」

AI 不会新建草稿文章。版本挂在原文章上，只有人应用版本后才会更新正文。已发布文章被 AI 直接应用版本需要 `publish` scope；草稿和笔记只需要 `write`。

### 人写，AI review

```text
写草稿
  -> 提交 Review
  -> AI 调 review_queue
  -> AI 调 content_get / revision_list
  -> AI 调 review_decide(approved / changes_requested, notes)
  -> 人查看 review_notes 后发布
```

### AI 写，人 review

```text
AI note_create / content_create
  -> AI review_submit
  -> 人在 /admin/review 查看
  -> 人批准、修改、发布
```

### 笔记转文章

- 在笔记编辑页点「转为文章」
- 默认生成草稿，原笔记保留
- 文章和笔记互相链接
- 笔记继续更新后，可点「同步最新内容到文章」
- AI 也可以调用 `note_promote`

### Wiki 链接

笔记正文里写：

```markdown
参见 [[另一篇笔记]] 和 [[rust-notes]]
```

保存时会解析并写入链接表。笔记详情会显示反向链接和出链。

---

## 安全说明

- 密码使用 Argon2id
- Session Token 数据库存 SHA-256，Cookie 为 HttpOnly + SameSite=Lax
- 后台写操作校验 CSRF
- 登录失败限流（5 次 / 10 分钟）
- Markdown 禁用原始 HTML，防止 XSS
- 评论纯文本转义，默认审核
- 上传限制大小和扩展名，随机文件名，禁止路径穿越
- MCP Token 哈希存储、scope、可吊销
- AI 默认不能发布；修改/删除已发布内容需要 `publish`，永久删除需要 `admin`
- 每次写入产生 revision，记录 actor
- 安全响应头：CSP、X-Frame-Options、Referrer-Policy 等

建议：

1. 首次登录后修改密码
2. 不要把 `MCP_TOKEN` 提交到 Git
3. 通过 HTTPS 暴露站点
4. 定期备份 `data/`
5. 给 AI 的 Token 默认不要勾选 `publish`

---

## 导入 devorz / Cactus 文章

仓库里带了一个一次性导入脚本，适合把 Typecho + Cactus 主题的公开文章迁移过来：

```bash
python3 -m venv /tmp/typecho-import-venv
/tmp/typecho-import-venv/bin/pip install -r scripts/requirements.txt

# 服务需要先运行，脚本通过 MCP 创建文章
MCP_TOKEN=your-token \
  /tmp/typecho-import-venv/bin/python scripts/import_devorz.py \
  --site https://www.devorz.com/ \
  --mcp-url http://127.0.0.1:3000/mcp \
  --token your-token \
  --db data/typecho.db
```

脚本会：

- 遍历分页的 `#post-list`，发现全部文章
- 抓取标题、日期、分类、关键词
- 把正文 HTML 转成 Markdown，保留代码块语言和表格
- 下载文章图片到 `data/uploads/imported/devorz/`，并改写为本地 `/media/...`
- 通过 MCP `content_create` 创建文章，再更新为已发布
- 在 `metadata` 里记录 `source_url`，方便追溯
- 按标题/缩略名跳过已导入文章

> 请只导入你拥有版权或已获得授权的内容。

## 开发

```bash
cargo check
cargo test
cargo clippy --all-targets
cargo run -- serve
```

项目结构：

```text
src/
  app.rs          AppState / Router 构建
  auth.rs         密码、Session、CSRF、限流
  config.rs       环境变量配置
  db.rs           SQLite 连接与迁移
  markdown.rs     Markdown、代码高亮、Wiki 链接
  mcp/            MCP 协议、工具、stdio/HTTP
  models.rs       领域模型
  repo/           内容、评论、标签、Token、修订、媒体
  seed.rs         首次启动种子数据
  settings.rs     站点设置
  storage.rs      Storage trait + 本地实现
  web/            公开站点 + 后台
templates/        Askama 模板
static/           CSS / JS
migrations/       SQLite 迁移
```

SQLite 使用 WAL、foreign_keys、busy_timeout。仓储层没有把 SQLite 语法散落到业务层，后续可以增加 PostgreSQL 适配。

---

## 路线图

- [x] 基础博客
- [x] 笔记与转文章
- [x] MCP stdio + HTTP
- [x] Review 工作流
- [x] Docker
- [ ] Typecho 数据导入
- [ ] S3/对象存储适配
- [ ] 全文搜索 FTS5 trigram
- [ ] 更完整的媒体管理（尺寸、封面）
- [ ] 多用户/角色
