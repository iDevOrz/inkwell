use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::OnceLock;

use pulldown_cmark::{
    CodeBlockKind, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html,
};
use syntect::highlighting::ThemeSet;
use syntect::html::highlighted_html_for_string;
use syntect::parsing::SyntaxSet;

#[derive(Debug, Clone)]
pub struct WikiTarget {
    pub label: String,
    pub href: Option<String>,
}

fn parser_options() -> Options {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    options
}

fn fence_language(info: &str) -> Option<String> {
    info.split_whitespace()
        .next()
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

const MAX_HEADING_ID_CHARS: usize = 100;

fn is_safe_heading_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= MAX_HEADING_ID_CHARS
        && id
            .chars()
            .all(|ch| ch.is_alphanumeric() || ch == '-' || ch == '_')
}

#[derive(Debug, Clone)]
pub struct ArticleHeading {
    pub id: String,
    pub label: String,
    pub level: u8,
}

pub fn render_markdown(body: &str, wiki: &HashMap<String, WikiTarget>) -> String {
    render(body, wiki, false).0
}

pub fn render_article(
    body: &str,
    wiki: &HashMap<String, WikiTarget>,
) -> (String, Vec<ArticleHeading>) {
    render(body, wiki, true)
}

fn render(
    body: &str,
    wiki: &HashMap<String, WikiTarget>,
    article: bool,
) -> (String, Vec<ArticleHeading>) {
    let body = body.replace("<!--more-->", "");
    let expanded = expand_wiki_links(&body, wiki);

    let parser = Parser::new_ext(&expanded, parser_options());
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
                    CodeBlockKind::Fenced(info) => fence_language(&info),
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
            Event::Start(Tag::Heading { level, id, .. }) => {
                // `{#id .class key=value}` is allowed for anchors only: classes and
                // arbitrary attributes (style=, data-*) could restyle the page.
                events.push(Event::Start(Tag::Heading {
                    level,
                    id: id.filter(|value| is_safe_heading_id(value)),
                    classes: Vec::new(),
                    attrs: Vec::new(),
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

    let headings = if article {
        article_headings(&mut events)
    } else {
        Vec::new()
    };
    let mut output = String::new();
    html::push_html(&mut output, events.into_iter());
    (output, headings)
}

fn article_headings(events: &mut [Event<'_>]) -> Vec<ArticleHeading> {
    // Reserve authored anchors, even when they occur after generated ones.
    let reserved: HashSet<String> = events
        .iter()
        .filter_map(|event| match event {
            Event::Start(Tag::Heading { id: Some(id), .. }) => Some(id.to_string()),
            _ => None,
        })
        .collect();
    let mut used = HashSet::new();
    let mut candidates = (1..).map(|index| format!("section-{index}"));
    let mut headings = Vec::new();
    let mut remaining = events;
    while let Some((event, rest)) = remaining.split_first_mut() {
        if let Event::Start(Tag::Heading { level, id, .. }) = event {
            if id.as_ref().is_some_and(|id| !used.insert(id.to_string())) {
                *id = None;
            }
            let level = *level as u8;
            if level <= HeadingLevel::H4 as u8 {
                let label = rest
                    .iter()
                    .take_while(|event| !matches!(event, Event::End(TagEnd::Heading(_))))
                    .filter_map(|event| match event {
                        Event::Text(text) | Event::Code(text) => Some(text.as_ref()),
                        Event::SoftBreak | Event::HardBreak => Some(" "),
                        _ => None,
                    })
                    .collect::<String>();
                if !label.trim().is_empty() {
                    let anchor = id.as_ref().map(ToString::to_string).unwrap_or_else(|| {
                        candidates
                            .find(|candidate| !reserved.contains(candidate))
                            .expect("unbounded heading IDs")
                    });
                    *id = Some(CowStr::from(anchor.clone()));
                    headings.push(ArticleHeading {
                        id: anchor,
                        label,
                        level,
                    });
                }
            }
        }
        remaining = rest;
    }
    headings
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

/// Uses the same parser and language rule as `render_markdown`, so a page
/// loads the Mermaid script exactly when a diagram is rendered.
pub fn contains_mermaid(body: &str) -> bool {
    Parser::new_ext(body, parser_options()).any(|event| {
        matches!(
            event,
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))
                if fence_language(&info).as_deref() == Some("mermaid")
        )
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
    let mut titles: Vec<String> = Vec::new();
    for link in wiki_link_spans(body) {
        if !titles.iter().any(|existing| existing == link.title) {
            titles.push(link.title.to_string());
        }
    }
    titles
}

fn expand_wiki_links(body: &str, wiki: &HashMap<String, WikiTarget>) -> String {
    let mut output = String::with_capacity(body.len());
    let mut cursor = 0;
    for link in wiki_link_spans(body) {
        output.push_str(&body[cursor..link.start]);
        let target = wiki
            .get(link.title)
            .or_else(|| wiki.get(&link.title.to_lowercase()));
        match target {
            Some(WikiTarget {
                label,
                href: Some(href),
            }) => {
                let label = label.replace('[', "\\[").replace(']', "\\]");
                let href = href.replace(')', "%29");
                output.push_str(&format!("[{label}]({href})"));
            }
            Some(WikiTarget { label, href: None }) => output.push_str(label),
            None => output.push_str(&format!("[[{}]]", link.title)),
        }
        cursor = link.end;
    }
    output.push_str(&body[cursor..]);
    output
}

struct WikiLinkSpan<'a> {
    start: usize,
    end: usize,
    title: &'a str,
}

/// Finds `[[title]]` links outside code. Code ranges come from the Markdown
/// parser itself, so nested/mixed fences, indented blocks and inline code
/// follow CommonMark exactly.
fn wiki_link_spans(body: &str) -> Vec<WikiLinkSpan<'_>> {
    let code_ranges = code_ranges(body);
    let in_code = |position: usize| code_ranges.iter().any(|range| range.contains(&position));

    let mut spans = Vec::new();
    let mut search_from = 0;
    while let Some(offset) = body[search_from..].find("[[") {
        let start = search_from + offset;
        let title_start = start + 2;
        let Some(close) = body[title_start..].find("]]") else {
            break;
        };
        let title_end = title_start + close;
        let raw_title = &body[title_start..title_end];
        if in_code(start) || raw_title.contains('\n') {
            search_from = title_start;
            continue;
        }
        let title = raw_title.trim();
        if !title.is_empty() {
            spans.push(WikiLinkSpan {
                start,
                end: title_end + 2,
                title,
            });
        }
        search_from = title_end + 2;
    }
    spans
}

fn code_ranges(body: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    for (event, range) in Parser::new_ext(body, parser_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) | Event::Code(_) => ranges.push(range),
            _ => {}
        }
    }
    ranges
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
    fn article_toc_links_use_unique_rendered_anchors() {
        let body = "## 介绍 **Flutter**\n\n### `build` {#section-1}\n\n## 重复 {#section-1}";
        let (html, headings) = render_article(body, &HashMap::new());

        assert_eq!(headings.len(), 3);
        assert_eq!(headings[0].label, "介绍 Flutter");
        assert_eq!(headings[1].label, "build");
        assert_eq!(headings[1].level, 3);
        assert_eq!(headings[1].id, "section-1");
        for heading in headings {
            assert_eq!(html.matches(&format!("id=\"{}\"", heading.id)).count(), 1);
        }
    }

    #[test]
    fn heading_attributes_cannot_inject_styles_or_classes() {
        let html = render_markdown(
            "# Login {#intro .overlay style=position:fixed onclick=x}",
            &HashMap::new(),
        );
        assert!(html.contains("id=\"intro\""));
        assert!(!html.contains("style"));
        assert!(!html.contains("overlay"));
        assert!(!html.contains("onclick"));
    }

    #[test]
    fn heading_ids_with_unsafe_characters_are_dropped() {
        let html = render_markdown("# Title {#bad\"id}", &HashMap::new());
        assert!(!html.contains("id="));
    }

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
    fn mermaid_detection_matches_rendering() {
        for body in ["``` mermaid\nA-->B\n```", "```Mermaid\nA-->B\n```"] {
            assert!(render_markdown(body, &HashMap::new()).contains(r#"class="mermaid""#));
            assert!(contains_mermaid(body), "{body}");
        }
        assert!(!contains_mermaid("````\n```mermaid\nA\n```\n````"));
    }

    #[test]
    fn wiki_titles_respect_nested_and_mixed_fences() {
        // A ``` line inside a ```` fence, or ~~~ inside ```, does not close it.
        let body = "````\n```\n[[InCode1]]\n````\n[[Real1]]\n```\n~~~\n[[InCode2]]\n```\n[[Real2]]";
        assert_eq!(
            wiki_titles(body),
            vec!["Real1".to_string(), "Real2".to_string()]
        );
    }

    #[test]
    fn wiki_titles_skip_indented_code() {
        let titles = wiki_titles("para\n\n    [[InCode]]\n\n[[Real]]");
        assert_eq!(titles, vec!["Real".to_string()]);
    }

    #[test]
    fn wiki_expansion_skips_nested_fences() {
        let mut wiki = HashMap::new();
        wiki.insert(
            "X".to_string(),
            WikiTarget {
                label: "X".to_string(),
                href: Some("/x".to_string()),
            },
        );
        let html = render_markdown("````\n```\n[[X]]\n````\n\n[[X]]", &wiki);
        assert_eq!(html.matches(r#"<a href="/x">"#).count(), 1, "{html}");
        assert!(html.contains("[[X]]"));
    }

    #[test]
    fn wiki_titles_skip_code() {
        let titles = wiki_titles("[[A]]\n```\n[[B]]\n```\n`[[C]]`");
        assert_eq!(titles, vec!["A".to_string()]);
    }
}
