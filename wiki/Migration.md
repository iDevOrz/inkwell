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
  --db data/inkwell.db
```

请在可信环境运行，并避免将 token 放进 shell 历史、日志或提交到 Git。

脚本会遍历分页文章，抓取标题、日期、分类和关键词，将正文 HTML 转为 Markdown，下载文章图片到本地，并记录来源 URL。按标题/slug 跳过已导入内容。

实际导入前，脚本先只读比较 `--db` 与 MCP 服务的 installation/runtime ID；匹配后再写入一个随机、立即清理的探针设置，要求 MCP 服务从同一数据库读到它。只有两步都通过，才会抓取文章或写入媒体/内容。请确保数据库路径指向 MCP 服务使用的同一份 Inkwell 数据，且 `--uploads` 指向该实例配置的 `DATA_DIR/uploads`。若从备份克隆出独立站点，应先为克隆实例重新生成 `settings.installation_id` 并启动服务。

可以先运行 `--dry-run` 预演。预演以只读方式打开数据库，不调用 MCP、不下载或保存图片，也不会写入媒体记录；预览 Markdown 会保留图片的源站 URL。

> 仅导入你拥有版权或已获授权的内容。
