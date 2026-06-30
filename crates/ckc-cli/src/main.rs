//! CKC CLI — Code Knowledge Compiler.
//!
//! ```text
//! ckc scan   <path>        Scan a repository and list source files
//! ckc build  <path>        Compile a repository into Knowledge IR
//! ckc query  callers ...   Query the compiled knowledge graph
//! ckc status [path]        Show build statistics
//! ```

use anyhow::Context;
use clap::{Parser, Subcommand};
use ckc_core::Compiler;
use std::path::{Path, PathBuf};

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
    },

    /// Query the compiled Knowledge IR.
    Query {
        /// Path to the SQLite database (or repository root if using default path).
        #[arg(short, long, default_value = ".")]
        db: PathBuf,

        #[command(subcommand)]
        sub: QuerySub,
    },

    /// Show build statistics for a compiled repository.
    Status {
        /// Path to the repository root or SQLite database.
        #[arg(default_value = ".")]
        path: PathBuf,
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
        /// Optional node kind filter (function, class, method, module, etc.).
        #[arg(short, long)]
        kind: Option<String>,
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
        Command::Build { path, db } => cmd_build(&path, db.as_deref()),
        Command::Query { db, sub } => cmd_query(&db, sub),
        Command::Status { path } => cmd_status(&path),
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

fn cmd_build(path: &PathBuf, db: Option<&Path>) -> anyhow::Result<()> {
    let compiler = Compiler::new(path);

    let db_path = match db {
        Some(p) => p.to_path_buf(),
        None => {
            let default_dir = path.join(".ckc");
            std::fs::create_dir_all(&default_dir)
                .context("creating .ckc directory")?;
            default_dir.join("ckc.db")
        }
    };

    println!("Building Knowledge IR for {} ...", path.display());
    let (_store, stats, result) = compiler.build(Some(&db_path))?;

    println!(
        "Done in {}ms — {} files scanned, {} parsed, {} failed",
        stats.duration_ms, stats.files_scanned, stats.files_parsed, stats.files_failed
    );
    println!(
        "  Nodes: {}  Edges: {}  DB: {}",
        stats.total_nodes,
        stats.total_edges,
        db_path.display()
    );

    // Print any diagnostics (parse errors)
    for diag in &result.diagnostics {
        let prefix = match diag.severity {
            ckc_ir::DiagnosticSeverity::Error => "ERROR",
            ckc_ir::DiagnosticSeverity::Warning => "WARNING",
        };
        eprintln!(
            "  {} [{}:{}] {}",
            prefix, diag.file_path, diag.line, diag.message
        );
    }

    Ok(())
}

fn cmd_query(db: &PathBuf, sub: QuerySub) -> anyhow::Result<()> {
    let db_path = resolve_db_path(db)?;
    let store = ckc_graph::GraphStore::open(&db_path)?;

    match sub {
        QuerySub::Callers { name, depth } => {
            let nodes = store.callers(&name, depth)?;
            print_nodes(&nodes, &format!("Callers of '{}'", name));
        }
        QuerySub::Callees { name, depth } => {
            let nodes = store.callees(&name, depth)?;
            print_nodes(&nodes, &format!("Callees of '{}'", name));
        }
        QuerySub::Imports { file } => {
            let edges = store.imports_of_file(&file)?;
            println!("Imports of '{}':", file);
            for e in &edges {
                println!("  → {}", e.target_id.name);
            }
        }
        QuerySub::Dependencies { name } => {
            let nodes = store.dependencies(&name)?;
            print_nodes(&nodes, &format!("Dependencies of '{}'", name));
        }
        QuerySub::Dependents { name } => {
            let nodes = store.dependents(&name)?;
            print_nodes(&nodes, &format!("Dependents of '{}'", name));
        }
        QuerySub::Neighbors { name, depth } => {
            let nodes = store.neighbors(&name, depth)?;
            print_nodes(&nodes, &format!("Neighbors of '{}' (depth {})", name, depth));
        }
        QuerySub::ListNodes { kind } => {
            let nodes = store.list_nodes(kind.as_deref())?;
            if let Some(k) = &kind {
                println!("Nodes of kind '{}':", k);
            } else {
                println!("All nodes:");
            }
            print_nodes(&nodes, "");
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

fn print_nodes(nodes: &[ckc_ir::IrNode], header: &str) {
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
