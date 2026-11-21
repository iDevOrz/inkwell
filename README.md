# Inkwell

自托管的个人博客与私密笔记工作台。Inkwell 把写作、知识整理和 AI 协作放在同一个地方；AI 提交的是附属于原文的待审版本，是否应用和发布由作者决定。

## 特性

- **博客**：文章、独立页面、分类、标签、搜索、归档与评论审核
- **私密笔记**：笔记工作台、`[[Wiki 链接]]`、反向链接，以及笔记转文章
- **AI 协作**：通过 MCP 读取内容、处理批注、生成版本、预览差异；默认不能发布
- **顺畅写作**：Markdown、服务端代码高亮、Mermaid 图表、分屏编辑与实时预览
- **完整发布**：RSS、Atom、sitemap 和 Open Graph
- **自托管**：Rust 单体应用、SQLite、本地媒体存储，无需前端构建链
- **一致的界面**：Light、Dark、System 三种主题

## 文档

详细安装、配置和工作流请看 [项目 Wiki](wiki/Home.md)：

- [安装与部署](wiki/Installation.md)
- [功能与编辑器](wiki/Features.md)
- [AI 协同改稿](wiki/AI-Collaboration.md)
- [MCP 配置与权限](wiki/MCP.md)
- [文章迁移](wiki/Migration.md)
- [开发指南](wiki/Development.md)

## 技术栈

Rust · Axum · Askama · SQLite · MCP

Inkwell 参考 Typecho 的数据模型，但不兼容 Typecho 主题或插件。
