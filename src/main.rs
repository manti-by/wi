//! `wi` — an MCP server over a directory of Markdown wiki pages.
//!
//! One binary, no configuration file: point it at a pages directory and it
//! serves three tools that let an agent search the wiki, read a page, and list
//! what is there. Search is BM25F across the title, metadata and body of every
//! page, with the corpus cached and invalidated on file changes.

mod config;
mod corpus;
mod error;
mod page;
mod search;
mod tools;

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use rmcp::ServiceExt;
use rmcp::transport::io::stdio;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tracing_subscriber::EnvFilter;

use crate::corpus::CorpusCache;
use crate::error::ServeError;
use crate::tools::WikiService;

/// The MCP protocol revision this server advertises.
pub const PROTOCOL_VERSION: rmcp::model::ProtocolVersion = rmcp::model::ProtocolVersion::LATEST;

/// Port the server listens on when none is given.
const DEFAULT_PORT: u16 = 4321;

/// Interface the server binds when none is given.
///
/// Loopback by default: this serves a local project's private notes, so it
/// should not be reachable from the network unless the operator asks for that.
const DEFAULT_HOST: &str = "127.0.0.1";

/// Pages directory used when `--wiki-path` is omitted.
const DEFAULT_WIKI_PATH: &str = "wiki/pages";

/// How a client reaches the server.
///
/// Stdio is the default because it is how MCP clients launch a local server:
/// the client owns the process, and no port, socket or `Host` header is
/// involved. HTTP is there for clients that connect to something already
/// running, or that cannot spawn a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Transport {
    /// Read JSON-RPC from stdin, write it to stdout. The default.
    Stdio,
    /// Serve Streamable HTTP on `--host` and `--port`.
    Http,
}

/// Serve a Markdown wiki over the Model Context Protocol.
#[derive(Debug, Parser)]
#[command(
    name = "wi",
    version,
    about = "Serve a Markdown wiki over MCP",
    long_about = "Serve a Markdown wiki over MCP.\n\n\
                  Exposes the wiki_search, wiki_get_page and wiki_list_pages tools. \
                  Search ranks pages with BM25F across each page's title, metadata and \
                  body.\n\n\
                  By default the server speaks MCP over stdin/stdout, which is how \
                  most clients launch a local server. Pass --transport http to serve \
                  Streamable HTTP instead."
)]
struct Args {
    /// How the server is reached.
    #[arg(long, value_enum, default_value_t = Transport::Stdio)]
    transport: Transport,

    /// Interface to bind. HTTP transport only.
    #[arg(long, default_value = DEFAULT_HOST, value_name = "HOST")]
    host: String,

    /// Port to bind. HTTP transport only.
    #[arg(long, default_value_t = DEFAULT_PORT, value_name = "PORT")]
    port: u16,

    /// Directory holding the `*.md` wiki pages.
    #[arg(long, default_value = DEFAULT_WIKI_PATH, value_name = "PATH")]
    wiki_path: PathBuf,

    /// Extra hostname or `host:port` to accept in the `Host` header.
    ///
    /// Only needed when `--host` binds somewhere other than loopback, so that
    /// clients reach the server by the address they were given rather than by
    /// spoofing `Host: localhost`. Repeatable. HTTP transport only.
    #[arg(long = "allowed-host", value_name = "HOST")]
    allowed_hosts: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    init_tracing()?;

    let root = resolve_wiki_path(&args.wiki_path)?;
    if !root.is_dir() {
        return Err(match root.is_file() {
            true => ServeError::NotADirectory(root).into(),
            false => ServeError::PagesNotFound(root).into(),
        });
    }

    match args.transport {
        Transport::Stdio => serve_stdio(root).await,
        Transport::Http => serve_http(args, root).await,
    }
}

/// Serves MCP over stdin/stdout until the client closes the stream.
async fn serve_stdio(root: PathBuf) -> anyhow::Result<()> {
    // stdout is the JSON-RPC channel here, which is why every log line goes to
    // stderr. One line from `println!` would corrupt the stream for the client.
    // The resolved path is worth logging: the client picks the working
    // directory, so the relative default can land somewhere else entirely.
    tracing::info!(wiki_path = %root.display(), "wi serving over stdio");

    let service = WikiService::with_cache(root, Arc::new(CorpusCache::new()));
    let running = service
        .serve(stdio())
        .await
        .map_err(|error| ServeError::Transport(Box::new(error)))?;

    running.waiting().await?;
    tracing::info!("wi stopped");
    Ok(())
}

/// Validates the HTTP options, then serves Streamable HTTP until shut down.
async fn serve_http(args: Args, root: PathBuf) -> anyhow::Result<()> {
    let address = wiki_host(&args.host)?;
    if !address.is_loopback() {
        // Nothing here authenticates a client, so an exposed bind is an
        // explicit decision that deserves to be noisy rather than silent.
        tracing::warn!(
            host = %args.host,
            "serving on a non-loopback interface with no authentication; \
             add --allowed-host for each name clients use to reach this address"
        );
    }

    serve(address, args.port, root, &args.host, &args.allowed_hosts).await
}

/// Resolves the pages root to an absolute path.
///
/// Making it absolute once, at startup, means later containment checks compare
/// two canonical paths regardless of the process's working directory.
fn resolve_wiki_path(path: &Path) -> anyhow::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    // Falls back to the unresolved path when the directory does not exist yet,
    // so the caller reports the path the operator actually typed.
    Ok(absolute.canonicalize().unwrap_or(absolute))
}

/// Parses `--host` into a socket address.
///
/// Bare hostnames resolve through DNS, so `localhost` and a machine's own name
/// both work; unresolvable names are reported before the server starts rather
/// than as a confusing bind failure.
fn wiki_host(host: &str) -> anyhow::Result<IpAddr> {
    use std::net::ToSocketAddrs;

    (host, 0)
        .to_socket_addrs()
        .map_err(|error| ServeError::Bind {
            host: host.to_string(),
            port: DEFAULT_PORT,
            source: error,
        })?
        .next()
        .map(|address| address.ip())
        .ok_or_else(|| {
            ServeError::Bind {
                host: host.to_string(),
                port: DEFAULT_PORT,
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "host did not resolve to an address",
                ),
            }
            .into()
        })
}

/// Starts the HTTP server and serves until it is shut down.
async fn serve(
    address: IpAddr,
    port: u16,
    root: PathBuf,
    requested_host: &str,
    extra_hosts: &[String],
) -> anyhow::Result<()> {
    let address = SocketAddr::new(address, port);
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|source| ServeError::Bind {
            host: address.ip().to_string(),
            port,
            source,
        })?;

    let bound = listener.local_addr()?;
    tracing::info!(%bound, wiki_path = %root.display(), "wi listening");

    // One cache behind every session handler, so N clients cost one parse of
    // the wiki rather than N.
    let cache = Arc::new(CorpusCache::new());

    let factory_root = root.clone();
    let factory_cache = Arc::clone(&cache);
    let service = StreamableHttpService::new(
        move || {
            Ok(WikiService::with_cache(
                factory_root.clone(),
                Arc::clone(&factory_cache),
            ))
        },
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_allowed_hosts(allowed_hosts(requested_host, extra_hosts)),
    );

    // MCP clients post to `/mcp`, but the transport owns its own routing and
    // discovery, so the service is mounted as a fallback rather than pinned to
    // a path a client might get wrong.
    let app = axum::Router::new().fallback_service(service);

    axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(ServeError::Serve)?;

    tracing::info!("wi stopped");
    Ok(())
}

/// Builds the transport's `Host` allowlist.
///
/// The transport rejects requests whose `Host` header is not on this list, which
/// is what stops a browser on another site from reaching a loopback server
/// through DNS rebinding. Deriving the list from `--host` matters: the default
/// list is loopback-only, so binding elsewhere without widening it would leave
/// the server simultaneously reachable by a spoofed `Host: localhost` from the
/// network *and* unreachable by the address the operator actually asked for.
fn allowed_hosts(host: &str, extra: &[String]) -> Vec<String> {
    let mut hosts = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];
    // A specific interface sends its own name; a wildcard binds every one, and
    // `0.0.0.0` is never a Host header a client would legitimately send.
    if !matches!(host, "0.0.0.0" | "::" | "[::]") && !hosts.iter().any(|h| h == host) {
        hosts.push(host.to_string());
    }
    for allowed in extra {
        if !hosts.contains(allowed) {
            hosts.push(allowed.clone());
        }
    }
    hosts
}

/// Resolves when the process is asked to stop.
async fn shutdown_signal() {
    let interrupt = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = interrupt => tracing::info!("received SIGINT, shutting down"),
        _ = terminate => tracing::info!("received SIGTERM, shutting down"),
    }
}

/// Initializes logging to stderr.
///
/// stdout is the JSON-RPC channel in stdio mode, so stderr is the only safe
/// home for logs: one line written to stdout corrupts the stream and the client
/// drops the session. The explicit `std::io::stderr` is there for that reason
/// alone — the subscriber's default writer is stdout.
fn init_tracing() -> Result<(), ServeError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .try_init()
        .map_err(ServeError::Logging)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_is_always_allowed() {
        let hosts = allowed_hosts("127.0.0.1", &[]);
        assert!(hosts.contains(&"localhost".to_string()));
        assert!(hosts.contains(&"127.0.0.1".to_string()));
        assert!(hosts.contains(&"::1".to_string()));
    }

    #[test]
    fn a_specific_interface_adds_its_own_name() {
        let hosts = allowed_hosts("192.168.1.5", &[]);
        assert!(hosts.contains(&"192.168.1.5".to_string()));
    }

    /// A wildcard is not a name any client would send, and it must not widen
    /// the loopback list on its own.
    #[test]
    fn a_wildcard_bind_does_not_add_itself() {
        for wildcard in ["0.0.0.0", "::"] {
            let hosts = allowed_hosts(wildcard, &[]);
            assert!(!hosts.contains(&wildcard.to_string()));
            assert_eq!(hosts.len(), 3);
        }
    }

    #[test]
    fn extra_allowed_hosts_are_added_without_duplicates() {
        let hosts = allowed_hosts("0.0.0.0", &["wiki.internal".into(), "wiki.internal".into()]);
        assert_eq!(hosts.iter().filter(|h| *h == "wiki.internal").count(), 1);
    }

    #[test]
    fn hostnames_and_addresses_both_resolve() {
        assert!(wiki_host("127.0.0.1").is_ok());
        assert!(wiki_host("0.0.0.0").is_ok());
        assert_eq!(wiki_host("192.0.2.7").unwrap().to_string(), "192.0.2.7");
        assert!(wiki_host("no-such-host.invalid").is_err());
    }
}
