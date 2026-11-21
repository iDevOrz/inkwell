use std::collections::HashMap;
use std::sync::OnceLock;

use pulldown_cmark::{CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd, html};
use syntect::highlighting::ThemeSet;
use syntect::html::highlighted_html_for_string;
use syntect::parsing::SyntaxSet;

#[derive(Debug, Clone)]
pub struct WikiTarget {
    pub label: String,
    pub href: Option<String>,
}

pub fn render_markdown(body: &str, wiki: &HashMap<String, WikiTarget>) -> String {
    let body = body.replace("<!--more-->", "");
    let expanded = expand_wiki_links(&body, wiki);

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);

    let parser = Parser::new_ext(&expanded, options);
    let mut events = Vec::new();
    let mut in_code = false;
    let mut code = String::new();
    let mut language: Option<String> = None;

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code = true;
                code.clear();
                language = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .map(|value| value.to_ascii_lowercase())
                        .filter(|value| !value.is_empty()),
                    CodeBlockKind::Indented => None,
                };
            }
            Event::End(TagEnd::CodeBlock) if in_code => {
                let rendered = if language.as_deref() == Some("mermaid") {
                    format!(
                        r#"<div class="mermaid">{}</div>"#,
                        crate::util::html_escape(&code)
                    )
                } else {
                    highlight_code(&code, language.as_deref())
                };
                events.push(Event::Html(CowStr::from(rendered)));
                in_code = false;
                language = None;
            }
            Event::Text(text) if in_code => code.push_str(&text),
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let dest_url = if is_safe_url(&dest_url) {
                    dest_url
                } else {
                    CowStr::from("#")
                };
                events.push(Event::Start(Tag::Link {
                    link_type,
                    dest_url,
                    title,
                    id,
                }));
            }
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let dest_url = if is_safe_url(&dest_url) {
                    dest_url
                } else {
                    CowStr::from("#")
                };
                events.push(Event::Start(Tag::Image {
                    link_type,
                    dest_url,
                    title,
                    id,
                }));
            }
            Event::Html(raw) | Event::InlineHtml(raw) => {
                // Never pass author-supplied raw HTML through. Turning it into text
                // makes pulldown-cmark escape it for us.
                events.push(Event::Text(raw));
            }
            _ if in_code => {}
            other => events.push(other),
        }
    }

    let mut output = String::new();
    html::push_html(&mut output, events.into_iter());
    output
}

pub fn excerpt(body: &str, max_chars: usize) -> String {
    let before_more = body.split("<!--more-->").next().unwrap_or(body);
    let source = if before_more.trim().is_empty() {
        body
    } else {
        before_more
    };

    let mut text = String::new();
    for event in Parser::new(source) {
        match event {
            Event::Text(value) | Event::Code(value) => text.push_str(&value),
            Event::SoftBreak | Event::HardBreak => text.push(' '),
            _ => {}
        }
    }

    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        normalized
    } else {
        let mut truncated = String::new();
        for (index, character) in normalized.chars().enumerate() {
            if index >= max_chars {
                truncated.push('…');
                break;
            }
            truncated.push(character);
        }
        truncated
    }
}

pub fn contains_mermaid(body: &str) -> bool {
    body.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("```mermaid") || line.starts_with("~~~mermaid")
    })
}

pub fn reading_minutes(body: &str) -> i64 {
    let characters = body
        .chars()
        .filter(|character| !character.is_whitespace())
        .count();
    ((characters as f64 / 400.0).ceil() as i64).max(1)
}

pub fn wiki_titles(body: &str) -> Vec<String> {
    let mut titles = Vec::new();
    let mut fenced = false;

    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }

        let mut inline_code = false;
        let characters: Vec<char> = line.chars().collect();
        let mut index = 0;
        while index < characters.len() {
            let character = characters[index];
            if character == '`' {
                inline_code = !inline_code;
                index += 1;
                continue;
            }
            if !inline_code
                && character == '['
                && characters.get(index + 1) == Some(&'[')
                && index + 2 < characters.len()
                && let Some(end) = characters[index + 2..]
                    .windows(2)
                    .position(|window| window == [']', ']'])
            {
                let end = index + 2 + end;
                let title: String = characters[index + 2..end].iter().collect();
                let title = title.trim();
                if !title.is_empty() && !titles.iter().any(|existing| existing == title) {
                    titles.push(title.to_string());
                }
                index = end + 2;
                continue;
            }
            index += 1;
        }
    }

    titles
}

fn expand_wiki_links(body: &str, wiki: &HashMap<String, WikiTarget>) -> String {
    let mut output = String::new();
    let mut fenced = false;

    for line in body.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            output.push_str(line);
            continue;
        }
        if fenced {
            output.push_str(line);
            continue;
        }

        let characters: Vec<char> = line.chars().collect();
        let mut inline_code = false;
        let mut index = 0;
        let mut cursor = 0;

        while index < characters.len() {
            let character = characters[index];
            if character == '`' {
                inline_code = !inline_code;
                index += 1;
                continue;
            }
            if !inline_code
                && character == '['
                && characters.get(index + 1) == Some(&'[')
                && index + 2 < characters.len()
                && let Some(end_offset) = characters[index + 2..]
                    .windows(2)
                    .position(|window| window == [']', ']'])
            {
                let end = index + 2 + end_offset;
                let title: String = characters[index + 2..end].iter().collect();
                let title = title.trim();

                output.extend(characters[cursor..index].iter());
                if let Some(target) = wiki.get(title).or_else(|| wiki.get(&title.to_lowercase())) {
                    match &target.href {
                        Some(href) => {
                            let label = target.label.replace('[', "\\[").replace(']', "\\]");
                            let href = href.replace(')', "%29");
                            output.push_str(&format!("[{label}]({href})"));
                        }
                        None => output.push_str(&target.label),
                    }
                } else {
                    output.push_str(&format!("[[{title}]]"));
                }

                cursor = end + 2;
                index = cursor;
                continue;
            }
            index += 1;
        }

        output.extend(characters[cursor..].iter());
    }

    output
}

fn is_safe_url(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return true;
    }
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with('/')
        || lower.starts_with('#')
        || lower.starts_with("./")
        || lower.starts_with("../")
    {
        return true;
    }
    !lower.contains(':')
}

fn highlight_code(code: &str, language: Option<&str>) -> String {
    let syntaxes = syntax_set();
    let themes = theme_set();

    let normalized = language.map(|value| match value.to_ascii_lowercase().as_str() {
        "js" => "javascript".to_string(),
        "ts" => "typescript".to_string(),
        "sh" | "shell" => "bash".to_string(),
        "yml" => "yaml".to_string(),
        other => other.to_string(),
    });

    let syntax = normalized
        .as_deref()
        .and_then(|token| syntaxes.find_syntax_by_token(token))
        .unwrap_or_else(|| syntaxes.find_syntax_plain_text());

    let theme = themes
        .themes
        .get("base16-ocean.dark")
        .or_else(|| themes.themes.get("Solarized (dark)"))
        .or_else(|| themes.themes.values().next());

    match theme {
        Some(theme) => match highlighted_html_for_string(code, syntaxes, syntax, theme) {
            Ok(html) => format!(r#"<div class="highlight">{html}</div>"#),
            Err(_) => plain_code(code, language),
        },
        None => plain_code(code, language),
    }
}

fn plain_code(code: &str, language: Option<&str>) -> String {
    let language = language
        .map(|value| format!(r#" class="language-{}""#, escape_attr(value)))
        .unwrap_or_default();
    format!(
        r#"<div class="highlight"><pre><code{language}>{}</code></pre></div>"#,
        crate::util::html_escape(code)
    )
}

fn escape_attr(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .collect()
}

fn syntax_set() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    // bat's extended syntax set covers Kotlin, Swift, Dart and many other
    // languages that syntect's small built-in set does not include.
    SYNTAXES.get_or_init(two_face::syntax::extra_newlines)
}

fn theme_set() -> &'static ThemeSet {
    static THEMES: OnceLock<ThemeSet> = OnceLock::new();
    THEMES.get_or_init(ThemeSet::load_defaults)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_html_is_escaped() {
        let html = render_markdown("<script>alert(1)</script>", &HashMap::new());
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn wiki_links_are_expanded() {
        let mut wiki = HashMap::new();
        wiki.insert(
            "Rust 笔记".to_string(),
            WikiTarget {
                label: "Rust 笔记".to_string(),
                href: Some("/admin/notes/1".to_string()),
            },
        );
        let html = render_markdown("参见 [[Rust 笔记]]", &wiki);
        assert!(html.contains(r#"<a href="/admin/notes/1">Rust 笔记</a>"#));
    }

    #[test]
    fn mermaid_blocks_are_rendered_as_diagrams() {
        let body = "```mermaid\nflowchart LR\n  A --> B\n```";
        let html = render_markdown(body, &HashMap::new());
        assert!(html.contains(r#"<div class="mermaid">"#));
        assert!(html.contains("flowchart LR"));
        assert!(contains_mermaid(body));
    }

    #[test]
    fn more_marker_is_removed_from_full_html() {
        let html = render_markdown("before\n\n<!--more-->\n\nafter", &HashMap::new());
        assert!(!html.contains("<!--more-->"));
        assert!(html.contains("before"));
        assert!(html.contains("after"));
    }

    #[test]
    fn excerpt_respects_more_marker() {
        let text = excerpt("第一段\n\n<!--more-->\n\n第二段", 100);
        assert!(text.contains("第一段"));
        assert!(!text.contains("第二段"));
    }

    #[test]
    fn unsafe_links_are_neutralized() {
        let html = render_markdown("[click](javascript:alert(1))", &HashMap::new());
        assert!(!html.contains("javascript:"));
    }

    #[test]
    fn wiki_titles_skip_code() {
        let titles = wiki_titles("[[A]]\n```\n[[B]]\n```\n`[[C]]`");
        assert_eq!(titles, vec!["A".to_string()]);
    }
}
