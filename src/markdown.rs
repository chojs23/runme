//! Markdown parsing focused on extracting runnable code blocks.
//!
//! The parser walks the event stream once, tracking heading context
//! and inline directives, and returns structured `CodeBlock` records
//! the CLI can later filter or execute.

use anyhow::{Result, anyhow};
use pulldown_cmark::{CodeBlockKind, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::Serialize;

/// Normalized metadata for a runnable block discovered in markdown.
#[derive(Clone, Debug, Serialize)]
pub struct CodeBlock {
    /// Stable identifier assigned by discovery order, e.g. `block-001`.
    pub id: String,
    /// Optional human-readable name provided via `runme:name`.
    pub name: Option<String>,
    /// Language info string (lowercase) when provided.
    pub language: Option<String>,
    /// Heading hierarchy providing context for reporter output.
    pub headings: Vec<String>,
    /// Raw contents stripped from the fenced block.
    pub content: String,
    /// Optional explanation when a directive marks the block as non-runnable.
    pub skip_reason: Option<String>,
}

impl CodeBlock {
    /// True when this block looks like a shell script that we can execute locally.
    pub fn is_shell(&self) -> bool {
        match self
            .language
            .as_deref()
            .map(|lang| lang.trim().to_ascii_lowercase())
        {
            Some(ref lang) if matches!(lang.as_str(), "bash" | "sh" | "shell" | "zsh") => true,
            None => true, // Missing info strings default to shell semantics for MVP.
            _ => false,
        }
    }
}

/// Parse markdown documents and surface runnable code blocks in discovery order.
pub fn extract_blocks(markdown: &str) -> Result<Vec<CodeBlock>> {
    let mut parser = Parser::new_ext(markdown, Options::all());
    let mut blocks = Vec::new();

    let mut heading_stack: Vec<Heading> = Vec::new();
    let mut active_heading: Option<HeadingBuilder> = None;
    let mut pending_skip: Option<String> = None;
    let mut pending_name: Option<String> = None;

    let mut collecting_block = false;
    let mut block_language: Option<String> = None;
    let mut block_content = String::new();

    let mut idx: usize = 0;

    while let Some(event) = parser.next() {
        match event {
            Event::Start(Tag::Heading { .. }) => {
                active_heading = Some(HeadingBuilder::new());
            }
            Event::Text(text) => {
                if let Some(builder) = active_heading.as_mut() {
                    builder.push(&text);
                } else if collecting_block {
                    block_content.push_str(&text);
                }
            }
            Event::Code(text) => {
                if collecting_block {
                    block_content.push_str(&text);
                } else if let Some(builder) = active_heading.as_mut() {
                    builder.push(&text);
                }
            }
            Event::Html(html) => {
                if is_skip_directive(&html) {
                    pending_skip = Some("Marked with runme:ignore".to_string());
                } else if let Some(name) = extract_name_directive(&html) {
                    pending_name = Some(name);
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                collecting_block = true;
                block_content.clear();
                block_language = match kind {
                    CodeBlockKind::Fenced(info) => normalize_info_string(&info),
                    CodeBlockKind::Indented => None,
                };
            }
            Event::End(TagEnd::Heading(level)) => {
                if let Some(builder) = active_heading.take() {
                    commit_heading(&mut heading_stack, builder, heading_depth(level));
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if !collecting_block {
                    return Err(anyhow!("encountered closing code block without start"));
                }

                idx += 1;
                let id = format!("block-{idx:03}");
                blocks.push(CodeBlock {
                    id,
                    name: pending_name.take(),
                    language: block_language.clone(),
                    headings: heading_stack.iter().map(|h| h.title.clone()).collect(),
                    content: block_content.trim().to_string(),
                    skip_reason: pending_skip.take(),
                });

                collecting_block = false;
                block_language = None;
                block_content.clear();
            }
            _ => {}
        }
    }

    anyhow::ensure!(!collecting_block, "markdown ended while inside code block");

    Ok(blocks)
}

#[derive(Clone, Debug)]
struct Heading {
    level: u32,
    title: String,
}

#[derive(Debug)]
struct HeadingBuilder {
    buffer: String,
}

impl HeadingBuilder {
    fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    fn push(&mut self, fragment: &CowStr) {
        if !self.buffer.is_empty() {
            self.buffer.push(' ');
        }
        self.buffer.push_str(fragment);
    }
}

fn commit_heading(stack: &mut Vec<Heading>, builder: HeadingBuilder, level: u32) {
    stack.retain(|existing| existing.level < level);
    stack.push(Heading {
        level,
        title: builder.buffer.trim().to_string(),
    });
}

fn heading_depth(level: HeadingLevel) -> u32 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn normalize_info_string(info: &CowStr) -> Option<String> {
    let raw = info.trim();
    if raw.is_empty() {
        return None;
    }
    Some(raw.split_whitespace().next().unwrap().to_ascii_lowercase())
}

fn is_skip_directive(html: &CowStr) -> bool {
    let normalized = html.trim().to_ascii_lowercase();
    normalized.contains("runme:ignore") || normalized.contains("runme:skip")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_basic_blocks() {
        let doc = r#"
# Heading

```
make test
```

```bash
cargo test
```
"#;

        let blocks = extract_blocks(doc).expect("parse");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].headings, vec!["Heading"]);
        assert!(blocks[0].language.is_none());
        assert_eq!(blocks[1].language.as_deref(), Some("bash"));
    }

    #[test]
    fn records_skip_directive() {
        let doc = r#"
<!-- runme:ignore -->
```bash
echo off
```
"#;
        let blocks = extract_blocks(doc).expect("parse");
        assert_eq!(
            blocks[0].skip_reason.as_deref(),
            Some("Marked with runme:ignore")
        );
    }

    #[test]
    fn captures_name_directive() {
        let doc = r#"
<!-- runme:name install-deps -->
```bash
echo hi
```
"#;
        let blocks = extract_blocks(doc).expect("parse");
        assert_eq!(blocks[0].name.as_deref(), Some("install-deps"));
    }
}

fn extract_name_directive(html: &CowStr) -> Option<String> {
    let raw = html.trim();
    if !raw.starts_with("<!--") || !raw.ends_with("-->") {
        return None;
    }
    let inner = raw
        .trim_start_matches("<!--")
        .trim_end_matches("-->")
        .trim();
    let lower = inner.to_ascii_lowercase();
    if !lower.starts_with("runme:name") {
        return None;
    }
    let name = inner[10..].trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}
