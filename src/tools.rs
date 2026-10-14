//! The MCP surface: three wiki tools over a cached corpus.
//!
//! Every tool answers with a single block of plain text rather than JSON,
//! because the consumers of this server are agents reading a result for a human
//! question — a numbered list of pages reads better than a nested object, and
//! keeps the output identical to the Python implementation this replaces.
//!
//! Failures are split in two: a *tool* error (`is_error: true`) tells the agent
//! something it can act on, like a missing argument or an unknown page, while an
//! empty or absent pages directory is reported plainly so an agent can go and
//! create pages.

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData as McpError,
    Implementation, InitializeResult, JsonObject, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler};
use serde_json::{Value, json};

use crate::config;
use crate::corpus::{Corpus, CorpusCache};
use crate::error::{ToolError, ToolResult};
use crate::page::{extract_snippets, page_title, resolve_page, summarize_meta};
use crate::search::tokenize;

/// Name reported to clients during the MCP handshake.
pub const SERVER_NAME: &str = "wi";

/// Narrows a `json!` literal into a tool input schema.
///
/// The schemas are compile-time constants, so a non-object here is a
/// programming mistake that should stop the process rather than reach a client.
fn schema(value: Value) -> Arc<JsonObject> {
    match value {
        Value::Object(map) => Arc::new(map),
        other => panic!("tool input schema must be a JSON object, got {other}"),
    }
}

/// The MCP server, holding the pages root and the corpus cache.
#[derive(Debug)]
pub struct WikiService {
    root: PathBuf,
    cache: CorpusCache,
}

impl WikiService {
    /// Creates a server that reads pages from `root`.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            cache: CorpusCache::new(),
        }
    }

    /// The directory this server reads pages from.
    #[cfg(test)]
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// The tool definitions, in listing order.
    ///
    /// The schemas are written out by hand rather than derived from handler
    /// signatures: clients pin against these exact names and shapes, and the
    /// descriptions are what steer an agent toward searching before guessing.
    pub fn available_tools() -> Vec<Tool> {
        vec![
            Tool::new(
                "wiki_search",
                "Search the project wiki of past debugging sessions, investigations, code reviews, and \
                 implementation notes. Consult this BEFORE answering questions about why something works \
                 the way it does, past decisions, or prior incidents. Returns ranked page names with \
                 snippets; fetch a full page with wiki_get_page.",
                schema(json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query (keywords or a question)",
                        },
                        "limit": {
                            "type": "integer",
                            "description": format!(
                                "Max results (default {}, max {})",
                                config::DEFAULT_LIMIT,
                                config::MAX_RESULTS
                            ),
                        },
                    },
                    "required": ["query"],
                })),
            ),
            Tool::new(
                "wiki_get_page",
                "Get the full Markdown content of a single wiki page by its file name, as returned by \
                 wiki_search or wiki_list_pages.",
                schema(json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Page file name, e.g. 2026-08-03-fix-mcp-server-2.0-api.md",
                        },
                    },
                    "required": ["name"],
                })),
            ),
            Tool::new(
                "wiki_list_pages",
                "List all project wiki pages with their metadata (title, type, date, status, services, \
                 areas, tags, tickets) without reading page bodies.",
                schema(json!({"type": "object", "properties": {}})),
            ),
        ]
    }

    /// Dispatches a tool call by name.
    pub fn call(&self, name: &str, arguments: &Value) -> ToolResult<String> {
        if !self.root.is_dir() {
            return Err(ToolError::PagesNotFound);
        }
        let arguments = match arguments {
            Value::Object(map) => map.clone(),
            _ => serde_json::Map::new(),
        };

        match name {
            "wiki_list_pages" => self.list_pages(),
            "wiki_search" => self.search(&arguments),
            "wiki_get_page" => self.get_page(&arguments),
            other => Err(ToolError::UnknownTool(other.to_string())),
        }
    }

    /// Lists every page with its metadata, without reading any body.
    fn list_pages(&self) -> ToolResult<String> {
        let corpus = self.cache.get(&self.root);
        if corpus.items.is_empty() {
            return Ok("No wiki pages found".to_string());
        }
        Ok(corpus
            .items
            .iter()
            .map(|page| {
                format!(
                    "{} — {} ({})",
                    page.name,
                    page_title(page),
                    summarize_meta(&page.meta)
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// Searches the corpus and formats the ranked hits.
    fn search(&self, arguments: &serde_json::Map<String, Value>) -> ToolResult<String> {
        let query = match arguments.get("query") {
            None | Some(Value::Null) => return Err(ToolError::MissingArgument("query")),
            Some(Value::String(query)) if query.is_empty() => {
                return Err(ToolError::MissingArgument("query"));
            }
            Some(Value::String(query)) => query.clone(),
            Some(_) => return Err(ToolError::NotAString("query")),
        };
        if query.chars().count() > config::MAX_QUERY_LENGTH {
            return Err(ToolError::QueryTooLong);
        }

        let limit = resolve_limit(arguments.get("limit"));
        let corpus = self.cache.get(&self.root);
        let terms = tokenize(&query);
        let hits = crate::corpus::rank(&corpus, &terms);
        if hits.is_empty() {
            return Ok(
                "No matching wiki pages. Try wiki_list_pages to browse the catalog.".to_string(),
            );
        }
        Ok(format_search_results(&corpus, &hits, limit))
    }

    /// Returns the raw text of one page, frontmatter included.
    fn get_page(&self, arguments: &serde_json::Map<String, Value>) -> ToolResult<String> {
        let name = match arguments.get("name") {
            None | Some(Value::Null) => return Err(ToolError::MissingArgument("name")),
            Some(Value::String(name)) if name.is_empty() => {
                return Err(ToolError::MissingArgument("name"));
            }
            Some(Value::String(name)) => name.clone(),
            Some(_) => return Err(ToolError::NotAString("name")),
        };

        let path =
            resolve_page(&self.root, &name).ok_or_else(|| ToolError::PageNotFound(name.clone()))?;
        Ok(std::fs::read_to_string(path)?)
    }
}

/// Clamps a `limit` argument into the configured range.
///
/// A missing or non-numeric limit falls back to the default, which keeps a
/// malformed call from turning into an error the agent has to interpret.
pub fn resolve_limit(arguments: Option<&Value>) -> usize {
    let raw = match arguments {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|float| float.trunc() as i64)),
        Some(Value::String(text)) => text.trim().parse::<i64>().ok(),
        _ => None,
    };
    let limit = raw.unwrap_or(config::DEFAULT_LIMIT as i64);
    limit.clamp(1, config::MAX_RESULTS as i64) as usize
}

/// Renders ranked hits as a numbered, snippet-bearing text block.
fn format_search_results(corpus: &Corpus, hits: &[crate::corpus::Ranked], limit: usize) -> String {
    hits.iter()
        .take(limit)
        .enumerate()
        .map(|(position, hit)| {
            let page = &corpus.items[hit.index];
            let snippets = extract_snippets(&page.body, &hit.terms);
            let snippet_lines = snippets
                .iter()
                .map(|snippet| format!("   > {snippet}"))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "{}. {} (score {:.3})\n   {} — {}\n{}",
                position + 1,
                page.name,
                hit.score,
                page_title(page),
                summarize_meta(&page.meta),
                snippet_lines,
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

impl ServerHandler for WikiService {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(crate::PROTOCOL_VERSION)
            .with_server_info(Implementation::new(
                SERVER_NAME,
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(format!(
                "Reads the Markdown wiki at {}. Use wiki_search to find pages, wiki_get_page to read \
one, and wiki_list_pages to browse the catalog.",
                self.root.display()
            ))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            tools: WikiService::available_tools(),
            ..ListToolsResult::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let arguments = request.arguments.map(Value::Object).unwrap_or(Value::Null);
        // A tool error is a result, not a protocol error: the agent needs to
        // read the message to know what went wrong and what to try instead.
        Ok(match self.call(&request.name, &arguments) {
            Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]).into(),
            Err(error) => {
                tracing::warn!(tool = %request.name, %error, "tool call failed");
                CallToolResult::error(vec![ContentBlock::text(error.to_string())]).into()
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn service_with(tag: &str, files: &[(&str, &str)]) -> WikiService {
        let root = std::env::temp_dir().join(format!("wi-tools-{}-{tag}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();
        for (name, contents) in files {
            std::fs::write(root.join(name), contents).unwrap();
        }
        WikiService::new(root)
    }

    fn cleanup(service: &WikiService) {
        let _ = std::fs::remove_dir_all(service.root());
    }

    #[test]
    fn lists_pages_with_title_and_summary() {
        let service = service_with(
            "list",
            &[(
                "relay.md",
                "---\ntitle: Relay mode\ntype: implementation\n---\nbody",
            )],
        );
        let text = service.call("wiki_list_pages", &json!({})).unwrap();
        assert_eq!(
            text,
            "relay.md — Relay mode (type: implementation, date: -, status: -)"
        );
        cleanup(&service);
    }

    #[test]
    fn an_empty_directory_is_not_an_error() {
        let service = service_with("empty", &[]);
        assert_eq!(
            service.call("wiki_list_pages", &json!({})).unwrap(),
            "No wiki pages found"
        );
        cleanup(&service);
    }

    #[test]
    fn a_missing_directory_is_a_tool_error() {
        let service = WikiService::new(PathBuf::from("/nonexistent/wi/pages"));
        assert!(matches!(
            service.call("wiki_list_pages", &json!({})),
            Err(ToolError::PagesNotFound)
        ));
    }

    #[test]
    fn search_ranks_the_relevant_page_first() {
        let service = service_with(
            "search",
            &[
                (
                    "relay.md",
                    "---\ntitle: Relay mode\n---\nthe relay mode is computed",
                ),
                ("pump.md", "---\ntitle: Pump\n---\nan unrelated page"),
            ],
        );
        let text = service
            .call("wiki_search", &json!({"query": "relay mode"}))
            .unwrap();
        assert!(text.starts_with("1. relay.md (score "), "got: {text}");
        cleanup(&service);
    }

    #[test]
    fn search_with_no_match_is_not_an_error() {
        let service = service_with("nomatch", &[("a.md", "---\ntitle: A\n---\nbody")]);
        let text = service
            .call("wiki_search", &json!({"query": "kubernetes"}))
            .unwrap();
        assert!(text.contains("No matching wiki pages"));
        cleanup(&service);
    }

    #[test]
    fn search_validates_its_query() {
        let service = service_with("validate", &[("a.md", "body")]);
        assert!(matches!(
            service.call("wiki_search", &json!({})),
            Err(ToolError::MissingArgument("query"))
        ));
        assert!(matches!(
            service.call("wiki_search", &json!({"query": 7})),
            Err(ToolError::NotAString("query"))
        ));
        assert!(matches!(
            service.call("wiki_search", &json!({"query": "x".repeat(501)})),
            Err(ToolError::QueryTooLong)
        ));
        cleanup(&service);
    }

    #[test]
    fn get_page_returns_the_whole_file() {
        let service = service_with("get", &[("relay.md", "---\ntitle: Relay\n---\n# Body\n")]);
        let text = service
            .call("wiki_get_page", &json!({"name": "relay"}))
            .unwrap();
        assert_eq!(text, "---\ntitle: Relay\n---\n# Body\n");
        assert!(matches!(
            service.call("wiki_get_page", &json!({"name": "../escape.md"})),
            Err(ToolError::PageNotFound(_))
        ));
        cleanup(&service);
    }

    #[test]
    fn an_unknown_tool_is_reported() {
        let service = service_with("unknown", &[("a.md", "body")]);
        assert!(matches!(
            service.call("wiki_nope", &json!({})),
            Err(ToolError::UnknownTool(name)) if name == "wiki_nope"
        ));
        cleanup(&service);
    }

    #[test]
    fn limit_is_clamped_into_range() {
        assert_eq!(resolve_limit(None), config::DEFAULT_LIMIT);
        assert_eq!(resolve_limit(Some(&json!("7"))), 7);
        assert_eq!(resolve_limit(Some(&json!("abc"))), config::DEFAULT_LIMIT);
        assert_eq!(resolve_limit(Some(&json!(0))), 1);
        assert_eq!(resolve_limit(Some(&json!(-5))), 1);
        assert_eq!(resolve_limit(Some(&json!(10000))), config::MAX_RESULTS);
    }

    #[test]
    fn every_tool_declares_an_object_schema() {
        for tool in WikiService::available_tools() {
            assert_eq!(tool.input_schema["type"], "object", "tool {}", tool.name);
            assert!(
                tool.description.is_some(),
                "tool {} has no description",
                tool.name
            );
        }
        assert_eq!(WikiService::available_tools().len(), 3);
    }
}
