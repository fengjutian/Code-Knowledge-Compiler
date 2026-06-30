//! CKC Knowledge IR — the core intermediate representation.
//!
//! This crate defines the foundational types used by every other crate in the
//! CKC workspace: nodes, edges, semantic metadata, and serialization.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Symbol Identity ────────────────────────────────────────────────────────

/// Globally-unique symbol identifier.
///
/// Stable across rebuilds: derived from file path, module path, name, and
/// signature hash rather than auto-increment IDs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SymbolId {
    /// Path relative to the repository root, e.g. `src/parser/lexer.rs`.
    pub file_path: String,
    /// Module path segments, e.g. `["parser", "lexer"]`.
    pub module_path: Vec<String>,
    /// The symbol's declared name.
    pub name: String,
    /// XXH3 hash of the symbol's signature (parameters, return type, etc.).
    /// Used to distinguish overloads and detect signature changes.
    pub signature_hash: u64,
}

impl SymbolId {
    /// Build a SymbolId from components.
    pub fn new(
        file_path: impl Into<String>,
        module_path: Vec<String>,
        name: impl Into<String>,
        signature_hash: u64,
    ) -> Self {
        Self {
            file_path: file_path.into(),
            module_path,
            name: name.into(),
            signature_hash,
        }
    }

    /// Serialize to a stable string key (for use as a database primary key).
    pub fn to_key(&self) -> String {
        format!(
            "{}::{}::{}::{:016x}",
            self.file_path,
            self.module_path.join("."),
            self.name,
            self.signature_hash
        )
    }

    /// Deserialize from a key produced by [`SymbolId::to_key`].
    pub fn from_key(key: &str) -> Option<Self> {
        let mut parts = key.splitn(4, "::");
        let file_path = parts.next()?.to_string();
        let module_path: Vec<String> = parts
            .next()?
            .split('.')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        let name = parts.next()?.to_string();
        let signature_hash = u64::from_str_radix(parts.next()?, 16).ok()?;
        Some(Self {
            file_path,
            module_path,
            name,
            signature_hash,
        })
    }
}

impl std::fmt::Display for SymbolId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.module_path.is_empty() {
            write!(f, "{}::{}", self.file_path, self.name)
        } else {
            write!(
                f,
                "{}::{}::{}",
                self.file_path,
                self.module_path.join("."),
                self.name
            )
        }
    }
}

// ── Node ───────────────────────────────────────────────────────────────────

/// The kind of a knowledge-graph node.
///
/// This enum is designed to be language-agnostic; individual language parsers
/// map their constructs into these variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    File,
    Module,
    Function,
    Method,
    Class,
    Struct,
    Enum,
    EnumVariant,
    Trait,
    TraitImpl,
    Interface,
    TypeAlias,
    Constant,
    Static,
    Variable,
}

/// Source position within a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLocation {
    pub line_start: u32,
    pub line_end: u32,
    pub col_start: u32,
    pub col_end: u32,
}

/// Visibility of a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    Private,
    Protected,
}

/// A node in the Knowledge IR graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrNode {
    pub id: SymbolId,
    pub kind: NodeKind,
    pub name: String,
    pub location: SourceLocation,
    pub visibility: Visibility,
    /// Arbitrary key-value metadata (language, owner, coverage, etc.).
    pub metadata: HashMap<String, serde_json::Value>,
    /// Semantic information populated by the semantic compiler (Phase 2+).
    pub semantic: Option<SemanticInfo>,
    /// Content hash of this node for incremental compilation (Phase 2+).
    pub hash: u64,
}

impl IrNode {
    pub fn new(id: SymbolId, kind: NodeKind, name: impl Into<String>, location: SourceLocation) -> Self {
        Self {
            id,
            kind,
            name: name.into(),
            location,
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            semantic: None,
            hash: 0,
        }
    }
}

// ── Edge ───────────────────────────────────────────────────────────────────

/// The kind of relationship between two nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// Function/method call.
    Calls,
    /// Module-level import.
    Imports,
    /// Structural containment (module contains function, class contains method).
    Contains,
    /// Class inheritance or trait implementation.
    Inherits,
    /// Object instantiation.
    Instantiates,
    /// General reference (variable usage, type reference).
    References,
    /// Module or file-level dependency.
    DependsOn,
}

/// A directed edge in the Knowledge IR graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrEdge {
    pub source_id: SymbolId,
    pub target_id: SymbolId,
    pub kind: EdgeKind,
    pub metadata: HashMap<String, serde_json::Value>,
}

impl IrEdge {
    pub fn new(source_id: SymbolId, target_id: SymbolId, kind: EdgeKind) -> Self {
        Self {
            source_id,
            target_id,
            kind,
            metadata: HashMap::new(),
        }
    }
}

// ── Semantic ───────────────────────────────────────────────────────────────

/// Semantic enrichment produced by the semantic compiler or LLM compiler.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SemanticInfo {
    /// One-sentence business purpose of this symbol.
    pub purpose: Option<String>,
    /// Multi-sentence summary.
    pub summary: Option<String>,
    /// Responsibility tags, e.g. "authentication", "logging".
    pub responsibility: Vec<String>,
    /// Business capability tags, e.g. "payment", "user-management".
    pub business_capability: Vec<String>,
    /// Design patterns detected, e.g. "Singleton", "Factory", "Strategy".
    pub design_pattern: Vec<String>,
    /// Quantitative complexity metrics.
    pub complexity: Option<ComplexityMetrics>,
    /// Identified risks.
    pub risks: Vec<RiskTag>,
}

/// Quantitative complexity measures for a symbol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityMetrics {
    /// Cyclomatic complexity (number of independent paths).
    pub cyclomatic: u32,
    /// Lines of code for this symbol.
    pub lines_of_code: u32,
    /// Number of distinct external callers.
    pub fan_in: u32,
    /// Number of distinct external callees.
    pub fan_out: u32,
}

/// A tagged risk on a symbol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskTag {
    pub severity: RiskSeverity,
    pub category: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskSeverity {
    Low,
    Medium,
    High,
    Critical,
}

// ── IR Build Result ────────────────────────────────────────────────────────

/// The output of a single compilation pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrBuildResult {
    /// Nodes extracted from the repository.
    pub nodes: Vec<IrNode>,
    /// Edges extracted from the repository.
    pub edges: Vec<IrEdge>,
    /// Number of files successfully parsed.
    pub files_parsed: u64,
    /// Number of files that failed to parse.
    pub files_failed: u64,
    /// Warnings or errors encountered during compilation.
    pub diagnostics: Vec<BuildDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildDiagnostic {
    pub file_path: String,
    pub line: u32,
    pub message: String,
    pub severity: DiagnosticSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Warning,
    Error,
}

// ── IR Version ─────────────────────────────────────────────────────────────

/// Current Knowledge IR version for forward-compatibility checks.
pub const IR_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_id_key_roundtrip() {
        let id = SymbolId::new("src/main.py", vec!["main".into()], "hello", 0xABCD);
        let key = id.to_key();
        let recovered = SymbolId::from_key(&key).unwrap();
        assert_eq!(id, recovered);
    }

    #[test]
    fn symbol_id_display() {
        let id = SymbolId::new("src/pkg/mod.py", vec!["pkg".into()], "process", 0);
        assert_eq!(id.to_string(), "src/pkg/mod.py::pkg::process");
    }
}
