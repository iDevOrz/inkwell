# Inkwell

[English](README.md) | [简体中文](README.zh-CN.md)

A self-hosted personal blog and private-notes workspace. Inkwell brings writing, knowledge organization, and AI collaboration together. AI edits are proposed as reviewable versions attached to the original content; the author decides whether to apply and publish them.

## Project origin

I’ve used Jekyll, Hexo, and Typecho for my blog, with Typecho being the system I used the longest. In recent years, security issues disclosed in PHP and its surrounding dependency ecosystem have made me reconsider how I want to maintain a blog over the long term. At the same time, traditional blog content models and editorial workflows can be difficult to integrate naturally with AI.

Around the time the “rewrite everything in Rust” trend became popular, I decided to follow along and build a blogging and note-taking system for my own use. Inkwell is not a Typecho-compatible implementation; it is a personal attempt to redesign the writing workflow around private notes and AI collaboration.

## Features

- **Blog:** posts, pages, categories, tags, search, archives, and comment moderation
- **Private notes:** inbox and evergreen workflows, wiki links, backlinks, and promotion to posts
- **AI collaboration:** MCP tools, inline annotations, reviewable content proposals, previews, and diffs; publishing stays under human control by default
- **Writing:** Markdown, server-side syntax highlighting, Mermaid diagrams, split-view editing, and live preview
- **Publishing:** RSS, Atom, sitemap, and Open Graph metadata
- **Self-hosted:** Rust application, SQLite, and local media storage with no frontend build pipeline
- **Themes:** Light, Dark, and System

## Documentation

See the [project wiki](wiki/Home.md) for detailed documentation:

- [Installation and deployment](wiki/Installation.md)
- [Features and editor](wiki/Features.md)
- [AI-assisted editing](wiki/AI-Collaboration.md)
- [MCP setup and permissions](wiki/MCP.md)
- [Content migration](wiki/Migration.md)
- [Development guide](wiki/Development.md)

## Tech stack

Rust · Axum · Askama · SQLite · MCP

Inkwell is inspired by Typecho's data model, but does not support Typecho themes or plugins.
