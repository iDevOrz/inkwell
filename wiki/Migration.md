# 导入 Typecho / Cactus 文章

仓库包含一次性脚本 `scripts/import_devorz.py`，用于从公开的 Typecho + Cactus 站点导入文章。

## 安装依赖

```bash
python3 -m venv .venv-import
.venv-import/bin/pip install -r scripts/requirements.txt
```

## 运行

Inkwell 服务需要先启动，并配置可写入草稿的 MCP token：

```bash
MCP_TOKEN=<token> \
  .venv-import/bin/python scripts/import_devorz.py \
  --site https://example.com/ \
  --mcp-url http://127.0.0.1:3000/mcp \
  --token <token> \
  --db data/typecho.db
```

请在可信环境运行，并避免将 token 放进 shell 历史、日志或提交到 Git。

脚本会遍历分页文章，抓取标题、日期、分类和关键词，将正文 HTML 转为 Markdown，下载文章图片到本地，并记录来源 URL。按标题/slug 跳过已导入内容。

> 仅导入你拥有版权或已获授权的内容。
