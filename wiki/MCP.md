# MCP 配置与权限

Inkwell 提供 stdio 和 HTTP 两种 MCP 传输。

## stdio

本地客户端可启动：

```bash
inkwell mcp
```

stdio 连接按本地进程信任处理，拥有完整权限。只应由可信的本地客户端启动。

## HTTP

HTTP MCP 随 Web 服务提供：

```text
POST /mcp
Authorization: Bearer <token>
Content-Type: application/json
```

Token 可通过 `MCP_TOKEN` 设置，也可在后台 `/admin/tokens` 创建独立 token。Token 只保存哈希，创建时显示一次。

## Scope

| Scope | 能力 |
|---|---|
| `read` | 读取文章、笔记、评论、统计与修订 |
| `write` | 创建/更新内容和笔记、创建版本、提交审核、移动到回收站 |
| `review` | 审核内容与评论 |
| `publish` | 发布文章/页面；修改或应用已发布内容 |
| `admin` | 全部权限 |

默认建议使用 `read,write,review`，仅在确实需要时授予 `publish` 或 `admin`。
评论审核 token 可以批准、隐藏或移入回收站评论；永久删除评论需要 `admin` scope。

## 主要工具

- 内容：`content_list`, `content_get`, `content_create`, `content_update`, `content_delete`, `content_publish`, `content_search`, `content_stats`
- 笔记：`note_list`, `note_get`, `note_create`, `note_update`, `note_promote`
- 审核：`review_queue`, `review_submit`, `review_decide`, `revision_list`, `revision_get`
- 批注：`annotation_queue`, `annotation_list`, `annotation_get`, `annotation_create`, `annotation_reply`, `annotation_resolve`
- AI 版本：`proposal_create`, `proposal_list`, `proposal_get`, `proposal_apply`, `proposal_reject`
- 评论：`comment_list`, `comment_moderate`

资源包括 `typecho://review/queue`、`typecho://annotations/queue`、`typecho://notes/recent` 和 `typecho://content/{id}`。

## 客户端配置示例

HTTP 客户端通常需要类似配置（字段以客户端文档为准）：

```json
{
  "mcpServers": {
    "inkwell": {
      "url": "https://example.com/mcp",
      "headers": {
        "Authorization": "Bearer <token>"
      }
    }
  }
}
```

stdio 客户端应配置 `inkwell` 的绝对路径、`args: ["mcp"]`，以及正确的 `DATA_DIR`。
