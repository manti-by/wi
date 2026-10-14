//! Wiki page parsing, presentation helpers and path resolution.
//!
//! A page is a Markdown file with optional YAML frontmatter. Everything the
//! tools show a human — the display title, the one-line metadata summary, the
//! search snippets — is derived here, so the three tools cannot disagree about
//! how a page reads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_yaml::Value;

use crate::config;
use crate::search::{Field, FieldCounts, count_field, line_hits};

/// The three searchable zones of a page, in scoring order.
pub const WIKI_FIELDS: [Field; 3] = [
    Field {
        weight: config::TITLE_BOOST,
        length_norm: config::TITLE_LENGTH_NORM,
    },
    Field {
        weight: config::METADATA_BOOST,
        length_norm: config::METADATA_LENGTH_NORM,
    },
    Field {
        weight: config::BODY_BOOST,
        length_norm: config::BODY_LENGTH_NORM,
    },
];

/// A parsed wiki page.
#[derive(Debug, Clone)]
pub struct Page {
    /// File name, which is the page's identity everywhere else.
    pub name: String,
    /// Frontmatter mapping; empty when the page has no frontmatter.
    pub meta: serde_json::Map<String, serde_json::Value>,
    /// Page text after the frontmatter block.
    pub body: String,
}

impl Page {
    /// Counts searchable tokens in the page's title, metadata and body zones.
    pub fn fields(&self) -> Vec<FieldCounts> {
        vec![
            count_field(&meta_string(&self.meta, "title").unwrap_or_default()),
            count_field(&metadata_text(&self.meta)),
            count_field(&self.body),
        ]
    }
}

/// Returns a frontmatter value rendered as a string, or `None` when it is
/// absent or falsy.
///
/// Falsy rather than merely empty, because the reference summarizer writes
/// `meta.get(key) or '-'` and `meta.get("title") or name`: a `title: 0` or
/// `status: false` falls back exactly the way an absent value does, and the
/// title in particular feeds the highest-weighted search zone.
fn meta_string(meta: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    match meta.get(key) {
        Some(value) if is_truthy(value) => Some(scalar_to_string(value)),
        _ => None,
    }
}

/// Renders a value the way Python's `str()` would.
///
/// Frontmatter reaches the agent's output through this function, so `None` has
/// to render as `None` rather than `null`, and a list or mapping sitting under
/// a scalar key has to render in Python's repr style.
fn scalar_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "None".to_string(),
        serde_json::Value::Bool(flag) => if *flag { "True" } else { "False" }.to_string(),
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(items) => format!(
            "[{}]",
            items.iter().map(python_repr).collect::<Vec<_>>().join(", ")
        ),
        serde_json::Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(key, value)| format!("'{}': {}", key, scalar_to_string(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Renders a string the way Python's `repr()` would, single-quoted.
fn python_repr(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => format!("'{text}'"),
        other => scalar_to_string(other),
    }
}

/// Returns `true` for values Python would treat as truthy.
fn is_truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(flag) => *flag,
        serde_json::Value::Number(number) => number.as_f64().is_none_or(|n| n != 0.0),
        serde_json::Value::String(text) => !text.is_empty(),
        serde_json::Value::Array(items) => !items.is_empty(),
        serde_json::Value::Object(map) => !map.is_empty(),
    }
}

/// Parses a wiki page file into metadata and body.
///
/// Returns `None` when the file cannot be read or its frontmatter is invalid
/// or is not a mapping; such pages are invisible to every tool rather than
/// surfacing as half-parsed entries.
pub fn parse_page_file(path: &Path) -> Option<Page> {
    let raw = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            tracing::warn!(page = %path.display(), %error, "skipping unreadable page");
            return None;
        }
    };

    let name = path.file_name()?.to_string_lossy().into_owned();

    // Pages are hand-edited and travel between machines, so CRLF has to be
    // normalized on the way in: it would otherwise shift every body offset,
    // every snippet's line number and the text `wiki_get_page` returns.
    let text = normalize_newlines(&raw);

    let Some((block, body_offset)) = split_frontmatter(&text) else {
        return Some(Page {
            name,
            meta: serde_json::Map::new(),
            body: text,
        });
    };

    let block = rescue_bare_dashes(block);
    let parsed = match parse_yaml_block(&block) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!(page = %name, %error, "skipping page with invalid frontmatter");
            return None;
        }
    };

    let meta = match parsed {
        Value::Null => serde_json::Map::new(),
        Value::Mapping(mapping) => yaml_mapping_to_json(mapping),
        _ => {
            tracing::warn!(page = %name, "skipping page with non-mapping frontmatter");
            return None;
        }
    };

    Some(Page {
        name,
        meta,
        body: text[body_offset..].to_string(),
    })
}

/// Rewrites CRLF and lone CR to LF.
fn normalize_newlines(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_string();
    }
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Splits leading frontmatter from the body.
///
/// Mirrors `\A---\s*\n(.*?)\n---\s*\n?` exactly, which has three consequences
/// that are easy to get wrong:
///
/// - the opening `---` is followed by whitespace *including* newlines, so a
///   blank line after it is part of the delimiter rather than the block;
/// - the closing `---` must be preceded by a newline, so `---\n---\nbody` is
///   **not** frontmatter — the first `---` cannot also close the block;
/// - the closing `---` is not required to end its line, so `---foo` closes it
///   and leaves `foo` at the start of the body.
fn split_frontmatter(text: &str) -> Option<(&str, usize)> {
    let after_open = text.strip_prefix("---")?;

    // `\s*` after the opening delimiter is greedy and backtracks one character
    // at a time when no closing `\n---` follows. Which length wins is therefore
    // load-bearing, and taking only the greedy one is wrong in both directions:
    //
    //   ---\n\n---\nbody  the greedy split finds no later `\n---`, so `\s*`
    //                       shrinks until the blank line's own newline can serve
    //                       as the delimiter's, giving empty frontmatter.
    //   ---\n---\nbody   the same backtrack would match an empty block, but there
    //                       is no second newline to give up, so the opening
    //                       delimiter cannot also close the block.
    //
    // Longest-first is the regex engine's own order.
    let run = after_open.len() - after_open.trim_start_matches(is_python_whitespace).len();
    for width in (0..=run).rev() {
        let Some(consumed) = after_open.get(..width) else {
            continue;
        };
        if !consumed.chars().all(is_python_whitespace) || !after_open[width..].starts_with('\n') {
            continue;
        }

        let rest = &after_open[width + 1..];
        let rest_offset = text.len() - rest.len();

        // `(.*?)` is lazy, so the earliest `\n---` at or after this split wins.
        let Some(closing) = rest.find("\n---") else {
            continue;
        };

        // `\s*\n?` after the closing fence is greedy with nothing to constrain
        // it, so it eats every blank line up to the first real content.
        let after = &rest[closing + 4..];
        let body = after.trim_start_matches(is_python_whitespace);
        let body_offset = rest_offset + closing + 4 + (after.len() - body.len());
        return Some((&rest[..closing], body_offset));
    }

    None
}

/// Whether a character is whitespace for a Python `\s` in a str pattern.
///
/// Rust's `char::is_whitespace` follows Unicode `White_Space`; Python's
/// `str.isspace`, which `\s` compiles to, additionally covers the four ASCII
/// separators below `\x20` and `NEL`.
fn is_python_whitespace(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '\u{1c}'..='\u{1f}' | '\u{85}')
}

/// Parses a frontmatter block, resolving repeated keys the way the reference
/// loader does.
///
/// A repeated key is a hard parse error to a strict YAML parser, which would
/// drop the page from every tool at once — a silent loss of a page that exists
/// on disk. The reference resolves duplicates last-wins, so a failed parse is
/// retried with repeats removed.
///
/// Only the *original* error is reported when the retry also fails, so
/// genuinely malformed frontmatter still skips the page instead of being
/// silently accepted in some mangled form.
fn parse_yaml_block(block: &str) -> Result<Value, serde_yaml::Error> {
    match serde_yaml::from_str::<Value>(block) {
        Err(error) => serde_yaml::from_str::<Value>(&dedupe_keys(block)).map_err(|_| error),
        other => other,
    }
}

/// Removes repeated mapping keys, keeping the last value for each.
///
/// Works on the raw block rather than on a parsed value, because a duplicate-key
/// error carries no position, and because a text pass can drop a key together
/// with its indented value block — dropping only the key line would orphan the
/// value and produce invalid YAML in turn.
///
/// Keys are scoped by their enclosing mapping, so `title:` repeated at the top
/// level and `title:` repeated inside a nested mapping are separate keys, as
/// they are to a YAML parser.
///
/// Two shapes are out of reach for a line-based pass and are therefore left to
/// the parser, which rejects them and skips the page: a repeat nested inside a
/// multi-line flow collection, and a repeat inside a list item's own mapping
/// (`items:\n  - name: a\n    name: b`). Both require frontmatter that is
/// already malformed and that no wiki page contains.
fn dedupe_keys(block: &str) -> String {
    let lines: Vec<&str> = block.split('\n').collect();
    let mut dropped = vec![false; lines.len()];

    // Indentation of each open mapping and the key path leading to it, so two
    // `title:` lines at different depths are not mistaken for a repeat.
    let mut open: Vec<(usize, String)> = Vec::new();
    let mut last_seen: HashMap<(Vec<String>, String), (usize, usize)> = HashMap::new();

    for (index, line) in lines.iter().enumerate() {
        let Some((key, indent)) = mapping_key(line) else {
            continue;
        };

        while open.last().is_some_and(|(depth, _)| *depth >= indent) {
            open.pop();
        }
        let scope: Vec<String> = open.iter().map(|(_, key)| key.clone()).collect();

        // The key's value runs until the next line indented no further. Blank
        // lines are carried along because they are not scope-defining.
        let mut end = index + 1;
        while end < lines.len() {
            let next = lines[end];
            let next_indent = next.len() - next.trim_start().len();
            if next.trim().is_empty() || next_indent > indent {
                end += 1;
            } else {
                break;
            }
        }

        // Drop the whole earlier entry, key line and indented value alike.
        if let Some(&(start, previous_end)) = last_seen.get(&(scope.clone(), key.to_string())) {
            for line in dropped.iter_mut().take(previous_end).skip(start) {
                *line = true;
            }
        }
        last_seen.insert((scope, key.to_string()), (index, end));

        open.push((indent, key.to_string()));
    }

    lines
        .iter()
        .enumerate()
        .filter(|(index, _)| !dropped[*index])
        .map(|(_, line)| *line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Returns the key and indentation of a block-mapping entry, or `None` for a
/// line that is blank, a comment, a sequence item, or a multi-line flow value.
fn mapping_key(line: &str) -> Option<(&str, usize)> {
    let indent = line.len() - line.trim_start().len();
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('-') || trimmed.starts_with('#') {
        return None;
    }

    let colon = trimmed.find(':')?;
    let key = &trimmed[..colon];
    if key.is_empty() || key.contains('#') {
        return None;
    }

    // A flow collection whose brackets balance on this line is a single-line
    // value, so the entry is exactly one line wide and safe to reason about.
    // One that spans lines is left to the parser.
    let rest = trimmed[colon + 1..].trim_start();
    if (rest.starts_with('[') || rest.starts_with('{')) && !brackets_balance(rest) {
        return None;
    }

    Some((key, indent))
}

/// Whether every bracket opened in a fragment is closed within it.
fn brackets_balance(fragment: &str) -> bool {
    let mut depth = 0i32;
    for ch in fragment.chars() {
        match ch {
            '[' | '{' => depth += 1,
            ']' | '}' => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return false;
        }
    }
    depth == 0
}

/// Quotes bare `-` values in a frontmatter block.
///
/// `branch: -` is legal YAML for null, but in a wiki it means the literal
/// string `-` ("no branch"), so it is rewritten before parsing.
fn rescue_bare_dashes(block: &str) -> String {
    block
        .split('\n')
        .map(rescue_bare_dash_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Rewrites a single `key: -` line, leaving every other line untouched.
fn rescue_bare_dash_line(line: &str) -> String {
    let indent_len = line.len() - line.trim_start().len();
    let (indent, rest) = line.split_at(indent_len);
    let Some(colon) = rest.find(':') else {
        return line.to_string();
    };
    let key = rest[..colon].trim_end();
    let after = &rest[colon + 1..];

    let mut key_chars = key.chars();
    let key_is_identifier = key_chars
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
    if !key_is_identifier || after.trim() != "-" {
        return line.to_string();
    }

    let gap = &after[..after.len() - after.trim_start().len()];
    let gap = if gap.is_empty() { " " } else { gap };
    format!("{indent}{key}:{gap}\"-\"")
}

/// Converts a YAML mapping into the JSON value model used for page metadata.
fn yaml_mapping_to_json(
    mapping: serde_yaml::Mapping,
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    for (key, value) in mapping {
        let key = match key {
            Value::String(text) => text,
            other => scalar_to_string(&yaml_to_json(&other)),
        };
        map.insert(key, yaml_to_json(&value));
    }
    map
}

/// Converts any YAML value into the JSON value model.
fn yaml_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(flag) => serde_json::Value::Bool(*flag),
        Value::Number(number) => number
            .as_i64()
            .map(serde_json::Value::from)
            .or_else(|| number.as_f64().map(serde_json::Value::from))
            .unwrap_or(serde_json::Value::Null),
        Value::String(text) => serde_json::Value::String(text.clone()),
        Value::Sequence(items) => {
            serde_json::Value::Array(items.iter().map(yaml_to_json).collect())
        }
        Value::Mapping(mapping) => serde_json::Value::Object(yaml_mapping_to_json(mapping.clone())),
        Value::Tagged(tagged) => yaml_to_json(&tagged.value),
    }
}

/// Flattens the searchable frontmatter fields into one lowercase string.
pub fn metadata_text(meta: &serde_json::Map<String, serde_json::Value>) -> String {
    let mut parts = Vec::new();
    for key in config::METADATA_KEYS {
        match meta.get(key) {
            Some(serde_json::Value::Array(items)) => {
                parts.extend(items.iter().map(scalar_to_string));
            }
            Some(value) if is_truthy(value) => parts.push(scalar_to_string(value)),
            _ => {}
        }
    }
    parts.join(" ").to_lowercase()
}

/// Renders a page's key frontmatter fields as a comma-separated summary.
///
/// Missing scalars render as `-` so the leading three fields are always there.
pub fn summarize_meta(meta: &serde_json::Map<String, serde_json::Value>) -> String {
    let scalar = |key: &str| meta_string(meta, key).unwrap_or_else(|| "-".to_string());
    let mut parts = vec![
        format!("type: {}", scalar("type")),
        format!("date: {}", scalar("date")),
        format!("status: {}", scalar("status")),
    ];
    for key in config::SUMMARY_KEYS {
        match meta.get(key) {
            Some(serde_json::Value::Array(items)) if !items.is_empty() => {
                let joined = items
                    .iter()
                    .map(scalar_to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                parts.push(format!("{key}: {joined}"));
            }
            Some(value) if is_truthy(value) => {
                parts.push(format!("{key}: {}", scalar_to_string(value)));
            }
            _ => {}
        }
    }
    parts.join(", ")
}

/// The page's display title, falling back to its file name.
pub fn page_title(page: &Page) -> String {
    meta_string(&page.meta, "title").unwrap_or_else(|| page.name.clone())
}

/// Picks the most relevant body lines to quote under a search hit.
///
/// Lines are scored by how many distinct query terms they contain, capped at
/// [`config::MAX_SNIPPETS`], then returned in line order. Line numbers are
/// relative to the body, so they stay meaningful after the frontmatter.
pub fn extract_snippets(body: &str, terms: &[String]) -> Vec<String> {
    let mut scored: Vec<(usize, usize, &str)> = Vec::new();
    for (index, line) in python_lines(body).into_iter().enumerate() {
        let stripped = line.trim();
        if stripped.is_empty() {
            continue;
        }
        let hits = line_hits(stripped, terms);
        if hits > 0 {
            scored.push((hits, index + 1, stripped));
        }
    }

    // Sort by hit count descending, keeping document order for ties, then put
    // the survivors back into line order.
    scored.sort_by_key(|item| std::cmp::Reverse(item.0));
    let mut top: Vec<(usize, usize, &str)> =
        scored.into_iter().take(config::MAX_SNIPPETS).collect();
    top.sort_by_key(|item| item.1);

    top.into_iter()
        .map(|(_, lineno, text)| {
            let truncated: String = text.chars().take(config::SNIPPET_LENGTH).collect();
            format!("L{lineno}: {truncated}")
        })
        .collect()
}

/// Splits text into lines the way Python's `str.splitlines()` does.
///
/// Rust's `str::lines` breaks on `\n` alone. Python also breaks on `\v`, `\f`,
/// the three ASCII information separators, `NEL`, and the Unicode line and
/// paragraph separators. Snippets are numbered by line, so the two have to
/// agree or a page containing one of those characters reports the wrong lines.
fn python_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();

    while let Some((index, ch)) = chars.next() {
        let is_break = ch == '\r'
            || ch == '\n'
            || matches!(ch, '\u{0b}'..='\u{0c}' | '\u{1c}'..='\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}');
        if !is_break {
            continue;
        }
        lines.push(&text[start..index]);
        let mut consumed = ch.len_utf8();
        // `\r\n` is one break, not two.
        if ch == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n') {
            chars.next();
            consumed += '\n'.len_utf8();
        }
        start = index + consumed;
    }
    // Python's splitlines does not yield a trailing empty line.
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Resolves a page name to a file path confined to the pages directory.
///
/// The `.md` suffix is optional and a leading `pages/` is stripped. Both sides
/// are canonicalized before the containment check, so neither `../` segments
/// nor a symlinked root can escape the directory.
pub fn resolve_page(pages_root: &Path, name: &str) -> Option<PathBuf> {
    let slug = name.trim().strip_prefix("pages/").unwrap_or(name.trim());
    let slug = if slug.ends_with(".md") {
        slug.to_string()
    } else {
        format!("{slug}.md")
    };

    let root = pages_root.canonicalize().ok()?;
    let target = root.join(slug).canonicalize().ok()?;
    target.starts_with(&root).then_some(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_temp(name: &str, contents: &str) -> PathBuf {
        // Each fixture gets its own directory so `page.name` is the bare file
        // name rather than a temp-dir-prefixed one.
        let mut path = std::env::temp_dir();
        path.push(format!("wi-page-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temp page dir");
        let file = path.join(name);
        std::fs::write(&file, contents).expect("write temp page");
        file
    }

    #[test]
    fn splits_frontmatter_from_body() {
        let path = write_temp(
            "split.md",
            "---\ntitle: Relay mode\ntype: implementation\n---\n# Body\n",
        );
        let page = parse_page_file(&path).expect("page parses");
        assert_eq!(page.name, "split.md");
        assert_eq!(page.meta.get("title"), Some(&json!("Relay mode")));
        assert_eq!(page.meta.get("type"), Some(&json!("implementation")));
        assert_eq!(page.body, "# Body\n");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_bare_dash_value_is_read_as_a_string() {
        let path = write_temp("dash.md", "---\nbranch: -\n---\nbody");
        let page = parse_page_file(&path).expect("page parses");
        assert_eq!(page.meta.get("branch"), Some(&json!("-")));
        assert_eq!(page.body, "body");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_page_without_frontmatter_keeps_its_whole_body() {
        let path = write_temp("plain.md", "# Just markdown\n");
        let page = parse_page_file(&path).expect("page parses");
        assert!(page.meta.is_empty());
        assert_eq!(page.body, "# Just markdown\n");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_duplicate_key_keeps_the_last_value_and_the_page() {
        let path = write_temp(
            "dup.md",
            "---\ntitle: First\ntitle: Second\ntags: [relay]\n---\nbody",
        );
        let page = parse_page_file(&path).expect("a repeated key must not make the page disappear");
        assert_eq!(page.meta.get("title"), Some(&json!("Second")));
        assert_eq!(page.meta.get("tags"), Some(&json!(["relay"])));
        assert_eq!(page.body, "body");
        let _ = std::fs::remove_file(path);
    }

    /// Dropping only the repeated key line would orphan an indented value and
    /// make the retry invalid, which drops the page all over again.
    #[test]
    fn a_duplicate_key_with_a_block_value_keeps_the_page() {
        let path = write_temp(
            "dupblock.md",
            "---\nservices:\n  - relay\n  - pump\nservices: manual\n---\nbody",
        );
        let page = parse_page_file(&path).expect("page survives a repeated block key");
        assert_eq!(page.meta.get("services"), Some(&json!("manual")));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_duplicate_key_inside_a_nested_mapping_keeps_the_page() {
        let path = write_temp(
            "nesteddup.md",
            "---\nmeta:\n  status: a\n  status: b\ntitle: X\n---\nbody",
        );
        let page = parse_page_file(&path).expect("page survives a repeated nested key");
        assert_eq!(page.meta.get("meta"), Some(&json!({"status": "b"})));
        assert_eq!(page.meta.get("title"), Some(&json!("X")));
        let _ = std::fs::remove_file(path);
    }

    /// The same key at two different depths is two different keys.
    #[test]
    fn keys_at_different_depths_are_not_repeats() {
        let path = write_temp(
            "depths.md",
            "---\ntitle: Outer\nmeta:\n  title: Inner\n---\nbody",
        );
        let page = parse_page_file(&path).expect("page parses");
        assert_eq!(page.meta.get("title"), Some(&json!("Outer")));
        assert_eq!(page.meta.get("meta"), Some(&json!({"title": "Inner"})));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_repeated_single_line_flow_value_keeps_the_page() {
        let path = write_temp(
            "flowdup.md",
            "---\ntags: [x, y]\ntags: [z]\ntype: debug\n---\nbody",
        );
        let page = parse_page_file(&path).expect("page survives a repeated flow list");
        assert_eq!(page.meta.get("tags"), Some(&json!(["z"])));
        assert_eq!(page.meta.get("type"), Some(&json!("debug")));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_repeated_block_sequence_keeps_the_page() {
        let path = write_temp(
            "seqdup.md",
            "---\nrelated:\n  - a\n  - b\nrelated:\n  - c\n---\nbody",
        );
        let page = parse_page_file(&path).expect("page survives a repeated block sequence");
        assert_eq!(page.meta.get("related"), Some(&json!(["c"])));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn frontmatter_that_is_only_invalid_still_skips_the_page() {
        let path = write_temp("bad.md", "---\ntitle: [unclosed\n---\nbody");
        assert!(parse_page_file(&path).is_none());
        // Invalid *and* repeated: the retry must not rescue it either.
        let path = write_temp("bad2.md", "---\na: 1\na: 2\nb: [x\n---\nbody");
        assert!(parse_page_file(&path).is_none());
        let _ = std::fs::remove_file(path);
    }

    /// The reference delimiter is `\A---\s*\n(.*?)\n---\s*\n?`, and each of
    /// these is a place where reading it as "a line containing exactly ---"
    /// gives a different answer.
    #[test]
    fn the_opening_delimiter_does_not_also_close_the_block() {
        let path = write_temp("emptyfm.md", "---\n---\nrelay in body\n");
        let page = parse_page_file(&path).expect("page parses");
        assert!(page.meta.is_empty());
        // No frontmatter at all, so the whole file is the body.
        assert_eq!(page.body, "---\n---\nrelay in body\n");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_closing_delimiter_need_not_end_its_line() {
        let path = write_temp("closetail.md", "---\ntitle: x\n---foo\nbody\n");
        let page = parse_page_file(&path).expect("page parses");
        assert_eq!(page.meta.get("title"), Some(&json!("x")));
        assert_eq!(page.body, "foo\nbody\n");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_indented_fence_does_not_close_the_block() {
        let path = write_temp("indent.md", "---\ntitle: x\n  ---\nbody\n");
        let page = parse_page_file(&path).expect("page parses");
        assert!(page.meta.is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_block_may_close_at_end_of_file() {
        let path = write_temp("eof.md", "---\ntitle: X\n---");
        let page = parse_page_file(&path).expect("page parses");
        assert_eq!(page.meta.get("title"), Some(&json!("X")));
        assert_eq!(page.body, "");
        let _ = std::fs::remove_file(path);
    }

    /// `\s*` is greedy but backtracks, so a blank line after the opener can
    /// supply the delimiter's newline. Reading it as "the longest whitespace
    /// run" instead shifts every body line number and indexes the `---` lines
    /// as body text, which perturbs the length normalization for every query.
    #[test]
    fn a_blank_line_after_the_opener_still_closes_the_block() {
        let path = write_temp("blankfm.md", "---\n\n---\n# Relay mode\n\nrelay text\n");
        let page = parse_page_file(&path).expect("page parses");
        assert!(page.meta.is_empty());
        assert_eq!(page.body, "# Relay mode\n\nrelay text\n");
        assert_eq!(
            extract_snippets(&page.body, &["relay".to_string()]),
            ["L1: # Relay mode", "L3: relay text"]
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_file_that_is_only_a_delimiter_has_no_frontmatter() {
        let path = write_temp("justfence.md", "---\n");
        let page = parse_page_file(&path).expect("page parses");
        assert!(page.meta.is_empty());
        assert_eq!(page.body, "---\n");
        let _ = std::fs::remove_file(path);
    }

    /// Every row is a `(file, block, body)` triple taken from the reference
    /// regex `\A---\s*\n(.*?)\n---\s*\n?`. `None` means no frontmatter at all.
    ///
    /// The interesting rows are the ones where reading the delimiter as "a line
    /// containing exactly ---" gives a different answer, and the ones where
    /// `\s*` has to backtrack rather than take the whole run.
    #[test]
    fn the_frontmatter_delimiter_matches_the_reference_regex() {
        type Expected<'a> = Option<(&'a str, &'a str)>;
        let cases: &[(&str, Expected)] = &[
            // The opening `---` cannot also close the block.
            ("---\n---\nrelay\n", None),
            // ...unless a blank line's own newline can serve as the delimiter's.
            (
                "---\n\n---\n# Relay mode\n\nrelay text\n",
                Some(("", "# Relay mode\n\nrelay text\n")),
            ),
            ("---\n\n\n---\nbody\n", Some(("", "body\n"))),
            // The closing fence need not end its line, nor be exactly `---`.
            (
                "---\ntitle: x\n---foo\nbody\n",
                Some(("title: x", "foo\nbody\n")),
            ),
            (
                "---\ntitle: X\n----\nbody\n",
                Some(("title: X", "-\nbody\n")),
            ),
            ("---\ntitle: X\n---", Some(("title: X", ""))),
            // An indented fence is not a fence.
            ("---\ntitle: x\n  ---\nbody\n", None),
            ("---\ntitle: X\n---\nbody", Some(("title: X", "body"))),
            // `\s*` backtracks off trailing spaces and tabs to find the newline.
            ("---\n   \n---\nbody\n", Some(("   ", "body\n"))),
            (
                "---\n title: X\ntype: impl\n---\nbody\n",
                Some((" title: X\ntype: impl", "body\n")),
            ),
            (
                "---\n\ttitle: X\n---\nbody\n",
                Some(("\ttitle: X", "body\n")),
            ),
            (
                "---   \ntitle: X\n---\nbody\n",
                Some(("title: X", "body\n")),
            ),
            ("---\n   ---\n\n\n\n---\n", Some(("   ---\n\n\n", ""))),
            // `\s*\n?` after the closing fence eats every blank line.
            (
                "---\ntitle: X\n---   \n\n\nbody\n",
                Some(("title: X", "body\n")),
            ),
            (
                "---\ntitle: X\n---\n\n\n\nbody\n",
                Some(("title: X", "body\n")),
            ),
            // Whitespace classes Rust's `char::is_whitespace` alone would miss. The
            // delimiter still closes here; treating these as ordinary content
            // characters would leave the page with no frontmatter at all.
            (
                "---\n\u{a0}\ntitle: X\n---\nbody\n",
                Some(("title: X", "body\n")),
            ),
            (
                "---\n\u{c}\ntitle: X\n---\nbody\n",
                Some(("title: X", "body\n")),
            ),
            // Unterminated or absent.
            ("---\n", None),
            ("no frontmatter\n", None),
        ];

        for (text, expected) in cases {
            match split_frontmatter(text) {
                None => assert!(expected.is_none(), "{text:?} should have no frontmatter"),
                Some((block, offset)) => {
                    let (want_block, want_body) = expected.expect("expected no frontmatter");
                    assert_eq!(block, want_block, "block of {text:?}");
                    assert_eq!(&text[offset..], want_body, "body of {text:?}");
                }
            }
        }
    }

    #[test]
    fn lines_split_the_way_python_splits_them() {
        // Python's splitlines also breaks on these; Rust's `lines` does not, and
        // snippet line numbers are counted off the split.
        assert_eq!(
            python_lines("a\nb\u{c}c\u{b}d\u{85}e\u{2028}f\r\ng"),
            ["a", "b", "c", "d", "e", "f", "g"]
        );
        assert_eq!(python_lines("trailing\n"), ["trailing"]);
        assert_eq!(python_lines(""), Vec::<&str>::new());
    }

    #[test]
    fn crlf_pages_are_normalized() {
        let path = write_temp(
            "crlf.md",
            "---\r\ntitle: X\r\n---\r\nrelay line\r\nsecond\r\n",
        );
        let page = parse_page_file(&path).expect("page parses");
        assert_eq!(page.meta.get("title"), Some(&json!("X")));
        // Body line numbers, and the text wiki_get_page returns, both depend on
        // this being LF-only.
        assert_eq!(page.body, "relay line\nsecond\n");
        assert_eq!(
            extract_snippets(&page.body, &["relay".into()]),
            ["L1: relay line"]
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn invalid_frontmatter_skips_the_page() {
        let path = write_temp("bad.md", "---\ntitle: [unclosed\n---\nbody");
        assert!(parse_page_file(&path).is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn non_mapping_frontmatter_skips_the_page() {
        let path = write_temp("scalar.md", "---\n- one\n- two\n---\nbody");
        assert!(parse_page_file(&path).is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn null_list_entries_render_as_none() {
        // `str(None)` is "None", and it becomes an indexed term too, so getting
        // this wrong changes scores rather than just display.
        let path = write_temp("nulllist.md", "---\ntickets:\n  - MNT-1\n  -\n---\nbody");
        let page = parse_page_file(&path).expect("page parses");
        assert_eq!(
            summarize_meta(&page.meta),
            "type: -, date: -, status: -, tickets: MNT-1, None"
        );
        assert!(metadata_text(&page.meta).contains("mnt-1 none"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn falsy_scalars_fall_back_the_way_they_do_in_the_reference() {
        // `meta.get(key) or '-'` is falsiness-based, and the title decides both
        // the display and the highest-weighted search zone.
        let path = write_temp("zero.md", "---\ntitle: 0\nstatus: false\n---\nbody");
        let page = parse_page_file(&path).expect("page parses");
        assert_eq!(page_title(&page), "zero.md");
        assert_eq!(summarize_meta(&page.meta), "type: -, date: -, status: -");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn metadata_text_covers_only_the_searchable_keys() {
        let meta = serde_json::Map::from_iter([
            ("tags".into(), json!(["relay", "mode"])),
            ("type".into(), json!("implementation")),
            ("status".into(), json!("resolved")),
            ("branch".into(), json!("master")),
        ]);
        assert_eq!(metadata_text(&meta), "relay mode implementation");
    }

    #[test]
    fn summarize_leads_with_three_scalars_using_dashes() {
        let meta = serde_json::Map::new();
        assert_eq!(summarize_meta(&meta), "type: -, date: -, status: -");
    }

    #[test]
    fn summarize_renders_list_fields_comma_joined() {
        let meta = serde_json::Map::from_iter([("tags".into(), json!(["relay", "mode"]))]);
        assert_eq!(
            summarize_meta(&meta),
            "type: -, date: -, status: -, tags: relay, mode"
        );
    }

    #[test]
    fn page_title_falls_back_to_the_file_name() {
        let page = Page {
            name: "relay.md".into(),
            meta: serde_json::Map::new(),
            body: String::new(),
        };
        assert_eq!(page_title(&page), "relay.md");
    }

    #[test]
    fn snippets_are_body_relative_capped_and_in_line_order() {
        let body = "alpha relay\nbeta\nrelay and mode\nrelay again\nrelay relay relay\n";
        let snippets = extract_snippets(body, &["relay".to_string()]);
        assert_eq!(snippets.len(), config::MAX_SNIPPETS);
        assert_eq!(snippets[0], "L1: alpha relay");
        assert_eq!(snippets[1], "L3: relay and mode");
    }

    #[test]
    fn snippets_truncate_long_lines() {
        let long = "relay ".repeat(80);
        let snippets = extract_snippets(&long, &["relay".to_string()]);
        assert_eq!(snippets.len(), 1);
        assert_eq!(
            snippets[0].chars().count(),
            "L1: ".len() + config::SNIPPET_LENGTH
        );
    }

    #[test]
    fn resolve_page_accepts_a_name_without_a_suffix() {
        let dir = std::env::temp_dir().join(format!("wi-resolve-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("relay.md"), "body").unwrap();
        assert!(resolve_page(&dir, "relay").is_some());
        assert!(resolve_page(&dir, "pages/relay.md").is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resolve_page_rejects_traversal() {
        let dir = std::env::temp_dir().join(format!("wi-traverse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(resolve_page(&dir, "../secret.md").is_none());
        assert!(resolve_page(&dir, "../../etc/passwd").is_none());
        assert!(resolve_page(&dir, "sub/../../escape.md").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
