//! Wiki page parsing, presentation helpers and path resolution.
//!
//! A page is a Markdown file with optional YAML frontmatter. Everything the
//! tools show a human — the display title, the one-line metadata summary, the
//! search snippets — is derived here, so the three tools cannot disagree about
//! how a page reads.

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
/// absent or empty.
fn meta_string(meta: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    match meta.get(key) {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => {
            let text = scalar_to_string(value);
            if text.is_empty() { None } else { Some(text) }
        }
    }
}

/// Renders a scalar the way Python's `str()` would, so summaries read the same.
fn scalar_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Bool(flag) => if *flag { "True" } else { "False" }.to_string(),
        serde_json::Value::Number(number) => number.to_string(),
        other => other.to_string(),
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
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            tracing::warn!(page = %path.display(), %error, "skipping unreadable page");
            return None;
        }
    };

    let name = path.file_name()?.to_string_lossy().into_owned();

    let Some((meta, body_offset)) = split_frontmatter(&text) else {
        return Some(Page {
            name,
            meta: serde_json::Map::new(),
            body: text,
        });
    };

    let block = rescue_bare_dashes(meta);
    let parsed: Value = match serde_yaml::from_str(&block) {
        Ok(value) => value,
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

/// Splits leading frontmatter from the body.
///
/// Mirrors `\A---\s*\n(.*?)\n---\s*\n?`: a file that does not open with a `---`
/// line has no frontmatter at all, and a file that opens one but never closes
/// it is left whole rather than being silently swallowed.
fn split_frontmatter(text: &str) -> Option<(&str, usize)> {
    let after_open = text.strip_prefix("---")?;
    // `\s*` tolerates trailing spaces before the opening delimiter's newline.
    let rest = after_open.trim_start_matches([' ', '\t', '\r']);
    let rest = rest.strip_prefix('\n')?;
    let rest_offset = text.len() - rest.len();

    // Find the closing delimiter. It must be the whole of its own line, and
    // the lazy `.*?` in the original pattern cannot reach past end-of-input,
    // so an unterminated block means the page simply has no frontmatter.
    let mut cursor = 0;
    let (block_end, line_end) = loop {
        let end = cursor + rest[cursor..].find('\n')?;
        if rest[cursor..end].trim() == "---" {
            break (cursor, end);
        }
        cursor = end + 1;
    };

    // `\s*\n?` after the closing delimiter eats the newline and any blank
    // lines that follow, so the body starts at the first real content.
    let after = &rest[line_end + 1..];
    let body = after.trim_start_matches([' ', '\t', '\r', '\n']);
    let body_offset = rest_offset + line_end + 1 + (after.len() - body.len());

    Some((&rest[..block_end], body_offset))
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
    for (index, line) in body.lines().enumerate() {
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
