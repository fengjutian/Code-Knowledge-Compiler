//! CKC CLI — Code Knowledge Compiler.
//!
//! ```text
//! ckc scan   <path>        Scan a repository and list source files
//! ckc build  <path>        Compile a repository into Knowledge IR
//! ckc query  callers ...   Query the compiled knowledge graph
//! ckc serve  <path>        Start HTTP API server
//! ckc mcp    <path>        Start MCP server (JSON-RPC over stdio)
//! ckc status [path]        Show build statistics
//! ```

mod mcp;

use anyhow::Context;
use clap::{Parser, Subcommand};
use ckc_core::Compiler;
use std::path::{Path, PathBuf};
use std::net::SocketAddr;

#[derive(Parser)]
#[command(
    name = "ckc",
    version,
    about = "Code Knowledge Compiler — compile code into queryable knowledge",
    long_about = "CKC compiles a code repository into a Knowledge IR (graph of nodes and edges) \
                  that can be queried structurally (callers, callees, imports, dependencies)."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan a repository and list discovered source files.
    Scan {
        /// Path to the repository root.
        path: PathBuf,
    },

    /// Build the Knowledge IR from a repository and persist to a database.
    Build {
        /// Path to the repository root.
        path: PathBuf,
        /// Optional path to the output SQLite database.
        /// Defaults to `<repo>/.ckc/ckc.db`.
        #[arg(short, long)]
        db: Option<PathBuf>,
        /// Force a full rebuild (ignore incremental cache).
        #[arg(long)]
        force: bool,
    },

    /// Query the compiled Knowledge IR.
    Query {
        #[arg(short, long, default_value = ".")]
        db: PathBuf,
        /// Output results as JSON.
        #[arg(long)]
        json: bool,

        #[command(subcommand)]
        sub: QuerySub,
    },

    /// Show build statistics for a compiled repository.
    Status {
        /// Path to the repository root or SQLite database.
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Start an HTTP API server for querying the knowledge graph.
    Serve {
        /// Path to the SQLite database or repository root.
        #[arg(default_value = ".")]
        db: PathBuf,
        /// Address to bind (default: 127.0.0.1:9876).
        #[arg(short, long, default_value = "127.0.0.1:9876")]
        addr: SocketAddr,
    },

    /// Start an MCP (Model Context Protocol) server over stdio.
    Mcp {
        /// Path to the SQLite database or repository root.
        #[arg(default_value = ".")]
        db: PathBuf,
    },
}

#[derive(Subcommand)]
enum QuerySub {
    /// Find callers of a symbol.
    Callers {
        /// Symbol name to query.
        #[arg(short, long)]
        name: String,
        /// Traversal depth (default: 1).
        #[arg(short, long, default_value = "1")]
        depth: u32,
    },
    /// Find callees of a symbol.
    Callees {
        #[arg(short, long)]
        name: String,
        #[arg(short, long, default_value = "1")]
        depth: u32,
    },
    /// List imports of a file.
    Imports {
        /// File path (relative to repo root).
        #[arg(short, long)]
        file: String,
    },
    /// Find dependencies of a symbol.
    Dependencies {
        #[arg(short, long)]
        name: String,
    },
    /// Find symbols that depend on (call) this symbol.
    Dependents {
        #[arg(short, long)]
        name: String,
    },
    /// List neighbors (both callers and callees).
    Neighbors {
        #[arg(short, long)]
        name: String,
        #[arg(short, long, default_value = "1")]
        depth: u32,
    },
    /// List all nodes, optionally filtered by kind.
    ListNodes {
        #[arg(short, long)]
        kind: Option<String>,
    },
    /// Full-text search on symbol names.
    Search {
        /// Search query (FTS5 syntax).
        #[arg(short, long)]
        query: String,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ckc=info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Scan { path } => cmd_scan(&path),
        Command::Build { path, db, force } => cmd_build(&path, db.as_deref(), force),
        Command::Query { db, sub, json } => cmd_query(&db, sub, json),
        Command::Status { path } => cmd_status(&path),
        Command::Serve { db, addr } => cmd_serve(&db, addr),
        Command::Mcp { db } => cmd_mcp(&db),
    }
}

// ── Commands ───────────────────────────────────────────────────────────────

fn cmd_scan(path: &PathBuf) -> anyhow::Result<()> {
    let scanner = ckc_core::Scanner::new(path);
    let files = scanner.scan()?;

    println!("Found {} source files in {}", files.len(), path.display());
    for f in &files {
        println!(
            "  [{}] {}",
            f.language.name(),
            f.path.strip_prefix(path).unwrap_or(&f.path).display()
        );
    }
    Ok(())
}

fn cmd_build(path: &PathBuf, db: Option<&Path>, force: bool) -> anyhow::Result<()> {
    let compiler = Compiler::new(path);

    let db_path = match db {
        Some(p) => p.to_path_buf(),
        None => {
            let default_dir = path.join(".ckc");
            std::fs::create_dir_all(&default_dir).context("creating .ckc directory")?;
            default_dir.join("ckc.db")
        }
    };

    let (total_nodes, total_edges) = if !force && db_path.exists() {
        println!("Building Knowledge IR for {} (incremental)...", path.display());
        let (_store, stats, _result, skipped) = compiler.build_incremental(&db_path)?;
        println!(
            "Done in {}ms — {} files scanned, {} changed, {} skipped, {} failed",
            stats.duration_ms, stats.files_scanned, stats.files_parsed, skipped, stats.files_failed
        );
        (stats.total_nodes, stats.total_edges)
    } else {
        println!("Building Knowledge IR for {} ...", path.display());
        let (_store, stats, result) = compiler.build(Some(&db_path))?;
        println!(
            "Done in {}ms — {} files scanned, {} parsed, {} failed",
            stats.duration_ms, stats.files_scanned, stats.files_parsed, stats.files_failed
        );
        for diag in &result.diagnostics {
            let prefix = match diag.severity {
                ckc_ir::DiagnosticSeverity::Error => "ERROR",
                ckc_ir::DiagnosticSeverity::Warning => "WARNING",
            };
            eprintln!("  {} [{}:{}] {}", prefix, diag.file_path, diag.line, diag.message);
        }
        (stats.total_nodes, stats.total_edges)
    };

    println!("  Nodes: {}  Edges: {}  DB: {}", total_nodes, total_edges, db_path.display());
    Ok(())
}

fn cmd_query(db: &PathBuf, sub: QuerySub, json: bool) -> anyhow::Result<()> {
    let db_path = resolve_db_path(db)?;
    let store = ckc_graph::GraphStore::open(&db_path)?;

    match sub {
        QuerySub::Callers { name, depth } => {
            let nodes = store.callers(&name, depth)?;
            output_nodes(&nodes, &format!("Callers of '{}'", name), json);
        }
        QuerySub::Callees { name, depth } => {
            let nodes = store.callees(&name, depth)?;
            output_nodes(&nodes, &format!("Callees of '{}'", name), json);
        }
        QuerySub::Imports { file } => {
            if json {
                let edges = store.imports_of_file(&file)?;
                println!("{}", serde_json::to_string_pretty(&edges)?);
            } else {
                let edges = store.imports_of_file(&file)?;
                println!("Imports of '{}':", file);
                for e in &edges {
                    println!("  → {}", e.target_id.name);
                }
            }
        }
        QuerySub::Dependencies { name } => {
            let nodes = store.dependencies(&name)?;
            output_nodes(&nodes, &format!("Dependencies of '{}'", name), json);
        }
        QuerySub::Dependents { name } => {
            let nodes = store.dependents(&name)?;
            output_nodes(&nodes, &format!("Dependents of '{}'", name), json);
        }
        QuerySub::Neighbors { name, depth } => {
            let nodes = store.neighbors(&name, depth)?;
            output_nodes(&nodes, &format!("Neighbors of '{}' (depth {})", name, depth), json);
        }
        QuerySub::ListNodes { kind } => {
            let nodes = store.list_nodes(kind.as_deref())?;
            output_nodes(&nodes, "", json);
        }
        QuerySub::Search { query } => {
            let nodes = store.search(&query)?;
            output_nodes(&nodes, &format!("Search '{}'", query), json);
        }
    }

    Ok(())
}

fn cmd_status(path: &PathBuf) -> anyhow::Result<()> {
    let db_path = resolve_db_path(path)?;

    if !db_path.exists() {
        println!(
            "No compiled database found at {}. Run `ckc build` first.",
            db_path.display()
        );
        return Ok(());
    }

    let store = ckc_graph::GraphStore::open(&db_path)?;
    let total_nodes = store.total_nodes();
    let total_edges = store.total_edges();

    println!("CK Database: {}", db_path.display());
    println!("  Total nodes: {}", total_nodes);
    println!("  Total edges: {}", total_edges);

    if let Some(version) = store.get_meta("ir_version") {
        println!("  IR version:  {}", version);
    }

    if total_nodes > 0 {
        println!("\n  By kind:");
        let counts = store.count_nodes()?;
        for (kind, count) in &counts {
            println!("    {:<16} {}", kind, count);
        }
    }

    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Resolve a database path: if it's a directory, look for `.ckc/ckc.db` inside.
fn resolve_db_path(path: &PathBuf) -> anyhow::Result<PathBuf> {
    if path.is_dir() {
        Ok(path.join(".ckc").join("ckc.db"))
    } else {
        Ok(path.clone())
    }
}

fn output_nodes(nodes: &[ckc_ir::IrNode], header: &str, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(nodes).unwrap_or_default());
        return;
    }
    if !header.is_empty() {
        println!("{} ({} results):", header, nodes.len());
    }
    for n in nodes {
        println!(
            "  [{kind}] {name} @ {file}:{line}",
            kind = ckc_graph::node_kind_str(n.kind),
            name = n.name,
            file = n.id.file_path,
            line = n.location.line_start
        );
    }
}

// ── HTTP API Server ────────────────────────────────────────────────────────

fn cmd_serve(db: &PathBuf, addr: SocketAddr) -> anyhow::Result<()> {
    let db_path = resolve_db_path(db)?;
    if !db_path.exists() {
        anyhow::bail!("Database not found at {}. Run `ckc build` first.", db_path.display());
    }

    println!("Starting CKC API server on http://{}", addr);
    println!("  GET /status         — build statistics");
    println!("  GET /nodes?kind=    — list nodes");
    println!("  GET /callers/:name  — find callers");
    println!("  GET /callees/:name  — find callees");
    println!("  GET /imports/:file  — list imports");
    println!("  GET /search?q=      — full-text search");

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let app_state = axum::Router::new()
            .route("/status", axum::routing::get(status_handler))
            .route("/nodes", axum::routing::get(nodes_handler))
            .route("/callers/{name}", axum::routing::get(callers_handler))
            .route("/callees/{name}", axum::routing::get(callees_handler))
            .route("/imports/{file}", axum::routing::get(imports_handler))
            .route("/search", axum::routing::get(search_handler))
            .layer(tower_http::cors::CorsLayer::permissive())
            .with_state(db_path.clone());

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app_state).await?;
        Ok::<_, anyhow::Error>(())
    })?;

    Ok(())
}

type AppState = PathBuf;

async fn open_store(state: &AppState) -> Result<ckc_graph::GraphStore, axum::http::StatusCode> {
    ckc_graph::GraphStore::open(state).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)
}

async fn status_handler(axum::extract::State(state): axum::extract::State<AppState>) -> axum::Json<serde_json::Value> {
    let store = open_store(&state).await.unwrap();
    axum::Json(serde_json::json!({
        "total_nodes": store.total_nodes(),
        "total_edges": store.total_edges(),
        "ir_version": store.get_meta("ir_version"),
    }))
}

async fn nodes_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::Json<serde_json::Value> {
    let store = open_store(&state).await.unwrap();
    let kind = params.get("kind").map(|s| s.as_str());
    let nodes = store.list_nodes(kind).unwrap_or_default();
    axum::Json(serde_json::json!(nodes))
}

async fn callers_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> axum::Json<serde_json::Value> {
    let store = open_store(&state).await.unwrap();
    let nodes = store.callers(&name, 1).unwrap_or_default();
    axum::Json(serde_json::json!(nodes))
}

async fn callees_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> axum::Json<serde_json::Value> {
    let store = open_store(&state).await.unwrap();
    let nodes = store.callees(&name, 1).unwrap_or_default();
    axum::Json(serde_json::json!(nodes))
}

async fn imports_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Path(file): axum::extract::Path<String>,
) -> axum::Json<serde_json::Value> {
    let store = open_store(&state).await.unwrap();
    let edges = store.imports_of_file(&file).unwrap_or_default();
    axum::Json(serde_json::json!(edges))
}

async fn search_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::Json<serde_json::Value> {
    let store = open_store(&state).await.unwrap();
    let q = params.get("q").map(|s| s.as_str()).unwrap_or("");
    let nodes = store.search(q).unwrap_or_default();
    axum::Json(serde_json::json!(nodes))
}

fn cmd_mcp(db: &PathBuf) -> anyhow::Result<()> {
    let db_path = resolve_db_path(db)?;
    if !db_path.exists() {
        anyhow::bail!("Database not found at {}. Run  first.", db_path.display());
    }
    let server = crate::mcp::McpServer::new(&db_path)?;
    server.run()
}

fn cmd_mcp(db: &PathBuf) -> anyhow::Result<()> {
    let db_path = resolve_db_path(db)?;
    if !db_path.exists() {
        anyhow::bail!("Database not found at {}. Run  first.", db_path.display());
    }
    let server = crate::mcp::McpServer::new(&db_path)?;
    server.run()
}
