# 开发指南

## 工具链

Rust 版本固定在仓库根目录的 `rust-toolchain.toml`，rustup 会自动选用。`Cargo.toml` 的 `rust-version` 是最低支持版本（MSRV，目前 1.94，受 sqlx 0.9 限制）。升级工具链时修改 `rust-toolchain.toml`，并在本地跑通 clippy。

## 构建与测试

```bash
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
cargo run -- serve
cargo llvm-cov --all-targets --summary-only   # 覆盖率（需安装 cargo-llvm-cov）
```

集成测试位于 `tests/`，每个文件一个主题，测试之间互不依赖。公共夹具在 `tests/common/mod.rs`：`TestApp` 可以直接创建任意角色的会话和指定权限的 token，提供表单、multipart 和 MCP 调用的辅助方法；`TestApp::with_file_database()` 改用 WAL 文件数据库和真实连接池，供并发测试使用。

| 文件 | 内容 |
|---|---|
| `access.rs` | 路由 × 角色权限矩阵、草稿与私密内容不外泄、CSRF |
| `admin_session.rs` | 真实的登录表单流程 |
| `admin_forms.rs` | 后台各表单的成功路径与校验错误 |
| `admin_media.rs` | 媒体上传（表单与编辑器）、校验、删除 |
| `public_site.rs` / `public_pages.rs` | 首页、静态资源、媒体可见性、搜索归档、读者评论 |
| `notes.rs` / `editing.rs` | 笔记工作流；批注、提案版本与冲突处理 |
| `mcp_tools.rs` | 所有 MCP 工具的权限范围与参数健壮性；新增工具必须在这里登记 |
| `mcp_protocol.rs` | JSON-RPC 协议、HTTP 鉴权、stdio 传输 |
| `concurrency.rs` | 多线程并发下的登录限流、slug、乐观锁、提案应用 |
| `migrations.rs` | 从每个旧版本数据库升级后数据完好 |
| `naming.rs` | 旧 `typecho` 标识的兼容（见 ADR 0001） |
| `repo.rs` | 直接测试仓储层 |
| `health.rs` | `/healthz` 与 `inkwell healthcheck` |

新增迁移时，`migrations.rs` 会自动覆盖“从上一个版本升级”的情况；如果迁移需要回填数据，请在该文件里为它补一个专门的测试。

## GitHub Actions

- **CI**：push / pull request 到 `main` 时，用固定的工具链检查格式、编译、测试和 Clippy；另有 `msrv` job 用 `rust-version` 指定的版本编译；Docker job 构建镜像，并通过 `/healthz` 和容器内的 `inkwell healthcheck` 验证启动与数据卷持久化。各 Rust job 用 `Swatinem/rust-cache` 缓存依赖和编译产物。
- **Coverage**：生成 LCOV 报告（短期 artifact），把覆盖率摘要写入 job summary；行覆盖率低于 `COVERAGE_MIN_LINES`（当前 85%）时 CI 失败。
- **Dependency audit**：Cargo 依赖变更时以及每周运行 RustSec advisory audit。`cargo-llvm-cov` 和 `cargo-audit` 通过 `taiki-e/install-action` 安装预编译版本。
- **Dependabot**：每周检查 Cargo 依赖、GitHub Actions 和 Docker 基础镜像更新，并创建更新 PR。Actions 与镜像都按 SHA / digest 固定。

Release 二进制和容器镜像发布尚未自动化；确定版本号策略与镜像发布目标后再添加发布 workflow。

## 项目结构

```text
src/
  app.rs          应用状态与路由构建
  auth.rs         密码、Session、CSRF、限流
  config.rs       环境变量配置（Config::from_lookup 便于测试）
  db.rs           SQLite 连接、迁移、写事务（BEGIN IMMEDIATE）
  health.rs       /healthz 探针
  markdown.rs     Markdown、代码高亮、Wiki 链接
  mcp/            MCP 协议、stdio/HTTP 传输
    tools/        每类工具一个文件；definitions.rs 是工具清单与参数 schema
  models.rs       领域模型
  repo/           评论、标签、Token、修订、批注、版本
    content/      内容仓储：queries / views / links / writes / promotion
  storage.rs      Storage trait + 本地存储实现
  web/
    admin/        后台：每个功能区一个文件，表单结构体在 forms.rs
    public/       公开站点：时间线、列表、文章、评论、feed、媒体
migrations/       SQLite 迁移
templates/        Askama 模板
static/           CSS、JavaScript 和本地第三方资源
tests/            集成测试（见上表）
```

SQLite 使用 WAL、外键和 busy timeout。会写入的事务统一用 `db::begin_write`（`BEGIN IMMEDIATE`）开启：普通 `BEGIN` 先读后写时，并发写入会直接得到 `SQLITE_BUSY`，而不会等待 busy timeout。仓储层集中封装 SQLite 查询，为后续增加 PostgreSQL 适配留出空间。

## 架构决策记录

重要的设计决定记录在 `wiki/adr/` 下：

- [ADR 0001：将遗留的 `typecho` 标识统一为 `inkwell`](adr/0001-rename-legacy-typecho-identifiers.md)

## 资源与许可

第三方前端资源随仓库本地托管；升级时请同步检查版本、许可证和 CSP 配置。Vditor 会用 XHR 取回 `static/vditor/dist/js/icons/ant.js` 并作为内联脚本注入，因此 CSP 中有该文件的 SHA-256 白名单（`src/web/mod.rs`）。升级 Vditor 后若 hash 不匹配，单元测试 `csp_allows_the_inlined_vditor_icon_script` 会失败并给出新值。
