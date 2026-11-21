# Inkwell Wiki

Inkwell 是一个自托管的个人博客与私密笔记工作台。文章和笔记保存在自己的服务器上，也可以通过 MCP 与 AI 协同整理和改稿。

## 文档

- [安装与部署](Installation.md)
- [功能与编辑器](Features.md)
- [AI 协同改稿](AI-Collaboration.md)
- [MCP 配置与工具](MCP.md)
- [导入 Typecho / Cactus 文章](Migration.md)
- [开发指南](Development.md)

## 项目原则

- 数据由站点所有者自行托管，SQLite 和本地媒体开箱即用。
- 笔记默认私密，成熟后可转为文章。
- AI 修改先生成附属于原文章的待审版本；人查看预览和差异后决定应用或拒绝。
- AI 默认不能直接发布；发布由人确认或通过显式授予 `publish` scope 完成。
