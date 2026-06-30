//! Core compilation orchestrator: scanner + parser dispatch + build pipeline.
//!
//! ckc-core drives the end-to-end compilation flow:
//!   1. Scan the repository for source files (`Scanner`)
//!   2. Dispatch each file to the appropriate language parser
//!   3. Resolve cross-file symbol references (`SymbolResolver`)
//!   4. Persist the Knowledge IR to the graph store
//!   5. Return build statistics

mod resolver;

use ckc_graph::GraphStore;
use ckc_ir::IrBuildResult;
use ckc_parser::{LanguageParser, ParseError, ParseResult, PythonParser};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[cfg(feature = "llm")]
use ckc_llm::SemanticCompiler;

// ── Scanner ────────────────────────────────────────────────────────────────

/// Detected language for a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Python,
}

impl Language {
    /// Detect language from a file extension.
    pub fn from_extension(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "py" | "pyi" | "pyx" => Some(Language::Python),
            _ => None,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Language::Python => "python",
        }
    }

    /// Common file extensions for this language.
    pub fn extensions(&self) -> &[&str] {
        match self {
            Language::Python => &["py", "pyi", "pyx"],
        }
    }
}

/// A scanned source file with detected language.
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: PathBuf,
    pub language: Language,
}

/// Repository scanner that discovers source files.
pub struct Scanner {
    repo_root: PathBuf,
}

impl Scanner {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
        }
    }

    /// Scan the repository and return all recognized source files.
    pub fn scan(&self) -> Result<Vec<SourceFile>, ignore::Error> {
        let mut files = Vec::new();

        for entry in ignore::Walk::new(&self.repo_root) {
            let entry = entry?;
            if !entry.file_type().map_or(false, |t| t.is_file()) {
                continue;
            }
            let path = entry.into_path();
            if let Some(lang) = Language::from_extension(&path) {
                files.push(SourceFile { path, language: lang });
            }
        }

        Ok(files)
    }

    /// Return the repository root.
    pub fn root(&self) -> &Path {
        &self.repo_root
    }
}

// ── Compiler ───────────────────────────────────────────────────────────────

/// Build statistics for a single compilation pass.
#[derive(Debug, Clone)]
pub struct BuildStats {
    pub files_scanned: u64,
    pub files_parsed: u64,
    pub files_failed: u64,
    pub total_nodes: u64,
    pub total_edges: u64,
    pub duration_ms: u64,
}

/// The main compiler that orchestrates scanning, parsing, and persistence.
pub struct Compiler {
    scanner: Scanner,
}

impl Compiler {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            scanner: Scanner::new(repo_root),
        }
    }

    /// Run a full build: scan → parse → persist.
    ///
    /// If `db_path` is `None`, an in-memory database is used (for quick exploration).
    /// Otherwise, results are persisted to a SQLite file.
    pub fn build(
        &self,
        db_path: Option<&Path>,
    ) -> Result<(GraphStore, BuildStats, IrBuildResult), anyhow::Error> {
        let start = Instant::now();

        let source_files = self.scanner.scan()?;
        let files_scanned = source_files.len() as u64;

        let store = match db_path {
            Some(p) => GraphStore::open(p)?,
            None => GraphStore::open_in_memory()?,
        };

        let python_parser = PythonParser::new();

        let mut total_nodes = 0u64;
        let mut total_edges = 0u64;
        let mut files_parsed = 0u64;
        let mut files_failed = 0u64;
        let mut all_nodes = Vec::new();
        let mut all_edges = Vec::new();
        let mut diagnostics = Vec::new();
        #[cfg(feature = "llm")]
        let mut source_snippets: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        for sf in &source_files {
            // Read source for LLM context (if feature enabled)
            #[cfg(feature = "llm")]
            {
                if let Ok(src) = std::fs::read_to_string(&sf.path) {
                    let rel = sf
                        .path
                        .strip_prefix(self.scanner.root())
                        .unwrap_or(&sf.path)
                        .display()
                        .to_string();
                    source_snippets.insert(rel, src);
                }
            }

            match self.parse_file(&python_parser, sf) {
                Ok(parse_result) => {
                    files_parsed += 1;
                    total_nodes += parse_result.nodes.len() as u64;
                    total_edges += parse_result.edges.len() as u64;
                    all_nodes.extend(parse_result.nodes);
                    all_edges.extend(parse_result.edges);
                }
                Err(e) => {
                    files_failed += 1;
                    diagnostics.push(ckc_ir::BuildDiagnostic {
                        file_path: sf.path.display().to_string(),
                        line: 0,
                        message: e.to_string(),
                        severity: ckc_ir::DiagnosticSeverity::Error,
                    });
                }
            }
        }

        // ── Cross-file symbol resolution ──────────────────────────────────
        let file_paths = resolver::collect_file_paths(&all_nodes);
        let sym_resolver = resolver::SymbolResolver::new(&all_nodes);
        let resolved_count = sym_resolver.resolve_calls(&mut all_edges, &file_paths);
        if resolved_count > 0 {
            tracing::info!("Resolved {} cross-file call(s)", resolved_count);
        }

        // ── LLM Semantic Enrichment (optional) ─────────────────────────────
        #[cfg(feature = "llm")]
        {
            if let Ok(provider) = ckc_llm::OpenAiProvider::from_env() {
                let llm_compiler = SemanticCompiler::new(provider);
                match llm_compiler.enrich_batch(&mut all_nodes, &source_snippets) {
                    Ok(count) => {
                        if count > 0 {
                            tracing::info!("LLM enriched {} node(s)", count);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("LLM enrichment failed: {}", e);
                    }
                }
            }
        }

        // Persist all collected nodes and edges
        store.persist_batch(&all_nodes, &all_edges)?;

        let duration_ms = start.elapsed().as_millis() as u64;

        let stats = BuildStats {
            files_scanned,
            files_parsed,
            files_failed,
            total_nodes,
            total_edges,
            duration_ms,
        };

        let build_result = IrBuildResult {
            nodes: all_nodes,
            edges: all_edges,
            files_parsed,
            files_failed,
            diagnostics,
        };

        Ok((store, stats, build_result))
    }

    fn parse_file(
        &self,
        parser: &dyn LanguageParser,
        source_file: &SourceFile,
    ) -> Result<ParseResult, ParseError> {
        parser.parse_file(self.scanner.root(), &source_file.path)
    }
}
