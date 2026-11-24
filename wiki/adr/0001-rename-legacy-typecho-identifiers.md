# ADR 0001：将遗留的 `typecho` 标识统一为 `inkwell`

- 状态：已采纳
- 日期：2026-11-24

## 背景

Inkwell 早期以 "Typecho-rs" 的名义开发，几个对外可见的标识沿用了 `typecho` 前缀：

| 标识 | 旧值 | 影响范围 |
|---|---|---|
| 会话 cookie | `typecho_session` | 所有已登录的浏览器 |
| 登录 CSRF cookie | `typecho_login_csrf` | 正在登录中的浏览器（15 分钟有效） |
| 默认数据库文件 | `<DATA_DIR>/typecho.db` | 所有未设置 `DATABASE_URL` 的部署 |
| MCP 资源 URI | `typecho://…` | 已配置资源 URI 的 MCP 客户端 |
| 主题偏好 | localStorage `typecho-theme` | 每个浏览器保存的明暗主题选择 |

README 已说明 Inkwell 不兼容 Typecho，这些名字容易让人误以为存在兼容关系。但直接改名会让已部署的实例出问题：所有人被登出、找不到原数据库（等同于空站点）、MCP 客户端读取失败、主题偏好丢失。

## 决定

新名字统一使用 `inkwell`，旧名字只读兼容，不再新发：

- **会话 cookie** 改为 `inkwell_session`。读取时先找新名字，找不到再读 `typecho_session`，因此升级后已登录的用户不会被登出。登录和退出时都会清除旧 cookie，浏览器最终只保留新名字。
- **登录 CSRF cookie** 改为 `inkwell_login_csrf`，不做兼容。它只在登录表单上存在 15 分钟，升级瞬间正在登录的人刷新页面即可。
- **默认数据库文件** 改为 `inkwell.db`。如果数据目录里只有 `typecho.db`，继续使用它，并在启动日志里提示手动改名；绝不自动改名，因为 SQLite 的 `-wal`/`-shm` 文件必须和主文件一起移动，在服务运行中改名可能丢数据。设置了 `DATABASE_URL` 的部署不受影响。导入脚本的 `--db` 默认值遵循同样的规则。
- **MCP 资源 URI** 改为 `inkwell://`。`resources/list` 和 `resources/templates/list` 只列出新前缀；`resources/read` 同时接受两种前缀。
- **主题偏好** 改为 `inkwell-theme`。首次读取时把旧值迁移到新键并删除旧键。

把 Typecho 作为**导入来源**的说明（导入脚本、迁移文档）保持不变，这些描述是准确的。

## 何时移除兼容代码

- `typecho_session` 回退：会话最长 365 天（`SESSION_DAYS` 上限），在本变更发布满一年后可以删除。
- `typecho://` 读取：在下一个大版本（1.0 或首个不兼容版本）的发布说明中预告后删除。
- `typecho.db` 回退与主题键迁移：代价很小，长期保留，直到确认没有旧部署。

删除时一并移除 `LEGACY_*` 常量及对应测试（`tests/naming.rs`、`config.rs` 与导入脚本中的回退测试）。

## 影响

- 已部署实例升级后无需任何操作即可继续工作；建议运维在方便时停服，把 `typecho.db`（及 `-wal`、`-shm`）改名为 `inkwell.db`。
- 新部署从一开始就只看到 `inkwell` 名称。
- 代码中多了少量 `LEGACY_*` 常量，集中在 `auth.rs`、`config.rs`、`mcp/tools.rs`、`static/theme-init.js` 和导入脚本里，搜索 `LEGACY` 即可找到全部兼容代码。
