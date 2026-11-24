# 安装与部署

## 本地运行

```bash
cp .env.example .env
```

首次运行建议在 `.env` 中设置 `ADMIN_PASSWORD`。未设置时，Inkwell 会生成随机密码并打印到启动日志。启动：

```bash
cargo run -- serve
```

默认地址为 <http://127.0.0.1:3000>，后台在 `/admin`。

## Docker

```bash
cp .env.example .env
```

至少填写强密码 `ADMIN_PASSWORD`。公网部署时设置外部 HTTPS 地址，例如 `SITE_URL=https://example.com`。`MCP_TOKEN` 可留空，之后可从后台创建独立 Token。

```bash
docker compose up -d --build
```

Compose 配置要求提供 `ADMIN_PASSWORD`，不会使用固定的管理员密码。数据保存在本地 `data/`，该目录已加入 `.gitignore`。Unix 系统启动时会将数据目录和媒体根目录权限收紧为仅服务账号可访问（`0700`）。
Docker Compose 默认只将服务端口绑定到 `127.0.0.1`，建议通过 HTTPS 反向代理对外提供服务。

Linux bind mount 使用容器 UID 10001。如果目录权限不允许写入，可调整宿主机目录所有者，或使用 Docker named volume。不要为了绕过权限问题将 `data/` 改成全局可读写；应让目录归容器服务 UID 所有。

## 反向代理与代理头

将 `SITE_URL` 设为浏览器访问的外部地址。HTTPS 部署时使用 `https://`，Inkwell 会据此默认启用 Secure Cookie。

默认**不信任**请求中的 `X-Forwarded-For` 和 `X-Real-IP`。只有当应用端口无法被外部直接访问，且反向代理会覆盖这些头时，才设置：

```text
TRUST_PROXY_HEADERS=true
```

启用后，Inkwell 取 `X-Forwarded-For` 的**最后一项**（即紧挨着应用的那层代理追加的地址），没有时再用 `X-Real-IP`。这要求应用前面只有一层代理；多层代理时，最外层之后的每一层都要覆盖而不是追加该头。

不要在应用端口仍可直接访问时启用此选项。

## 环境变量

| 变量 | 默认值 | 说明 |
|---|---|---|
| `BIND_ADDR` | `127.0.0.1:3000` | 监听地址；Docker 镜像内为 `0.0.0.0:3000` |
| `DATA_DIR` | `./data` | SQLite 数据库和上传文件目录 |
| `DATABASE_URL` | `sqlite://<DATA_DIR>/typecho.db` | 数据库连接串 |
| `SITE_URL` | `http://127.0.0.1:3000` | 对外站点地址 |
| `ADMIN_USER` | `admin` | 首次启动创建的用户名 |
| `ADMIN_PASSWORD` | 随机生成 | 首次启动的管理员密码 |
| `MCP_TOKEN` | 空 | 可选的部署级 HTTP MCP token |
| `MCP_TOKEN_SCOPES` | `read,write,review` | 部署级 MCP token 权限；可选 read、write、publish、review、admin，写错会导致启动失败 |
| `COOKIE_SECURE` | 根据 `SITE_URL` 自动判断 | 强制 Secure Cookie |
| `TRUST_PROXY_HEADERS` | `false` | 是否信任反向代理 IP 头 |
| `SESSION_DAYS` | `14` | 登录有效期（天），取值 1–365 |
| `MAX_UPLOAD_BYTES` | `10485760` | 单个上传文件的大小上限 |
| `RUST_LOG` | `inkwell=info,tower_http=info` | 日志级别 |

站点标题、简介、时区、评论设置和每页数量在 `/admin/settings` 管理。

## 备份

停止服务后，可备份整个 `data/` 目录。也可以使用 SQLite 在线备份：

```bash
sqlite3 data/typecho.db ".backup 'data/backup.db'"
```
