# 开发指南

## 构建与测试

```bash
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
cargo run -- serve
```

## 项目结构

```text
src/
  app.rs          应用状态与路由构建
  auth.rs         密码、Session、CSRF、限流
  config.rs       环境变量配置
  db.rs           SQLite 连接与迁移
  markdown.rs     Markdown、代码高亮、Wiki 链接
  mcp/            MCP 协议、工具、stdio/HTTP
  models.rs       领域模型
  repo/           内容、评论、标签、Token、修订、批注、版本
  storage.rs      Storage trait + 本地存储实现
  web/            公开站点与后台
migrations/       SQLite 迁移
templates/        Askama 模板
static/           CSS、JavaScript 和本地第三方资源
tests/            集成 smoke tests
```

SQLite 使用 WAL、外键和 busy timeout。仓储层集中封装 SQLite 查询，为后续增加 PostgreSQL 适配留出空间。

## 资源与许可

第三方前端资源随仓库本地托管；升级时请同步检查版本、许可证和 CSP 配置。Vditor 图标脚本使用 CSP SHA-256 白名单，升级相关静态文件后需重新核对 hash。
