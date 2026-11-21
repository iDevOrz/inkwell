# 功能与编辑器

## 博客

- 文章、独立页面、草稿、发布与回收站
- 分类、标签、搜索、时间归档
- 评论与后台审核
- RSS 2.0、Atom、sitemap、robots.txt 和 Open Graph
- 首页按时间线展示文章，并按年份标记；滚动时每批加载 100 篇

Inkwell 不兼容 Typecho 主题或插件，也不提供 XML-RPC、多站点、自定义字段或文章密码保护。

## 私密笔记

笔记是独立于公开文章的私密内容，有 Inbox、Developing、Evergreen、Archived 等状态。正文支持 `[[笔记标题]]` Wiki 链接和反向链接。成熟的笔记可转成文章，也可以同步后续修改。

## Markdown 与图表

Markdown 在服务端渲染，代码使用 syntect 高亮。原始 HTML 会转义，链接和图片 URL 会检查危险 scheme。

Mermaid 使用 fenced code block：

````markdown
```mermaid
classDiagram
  Note --> Post : promote
```
````

支持浅色、深色主题；图表字号保持可读，宽图可横向滚动。渲染采用 Mermaid strict security level。

## 编辑器

后台使用 Vditor 4.0.1 分屏模式：

- 左侧编辑 Markdown，右侧实时预览
- 工具栏、大纲、快捷键、表格、代码块与媒体上传
- 预览使用同一套服务端 Markdown 渲染，尽量保持预览与发布结果一致
- Vditor 静态资源随应用部署，不依赖公共 CDN

## 媒体

媒体默认保存在本地 `data/uploads/`。应用使用 `Storage` trait 抽象存储层，未来可以增加对象存储实现。上传大小和扩展名受限，文件名随机生成。

## 外观

前台、后台与登录页支持 Light、Dark、System 三种主题。样式采用 shadcn/ui 的 CSS token 风格，不引入 React 或 Tailwind 构建链。
