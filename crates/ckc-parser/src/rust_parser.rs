//! Rust language parser using tree-sitter-rust.
//!
//! Extracts Knowledge IR nodes and edges from Rust source files:
//! functions, structs, enums, traits, impls, modules, methods, and
//! their relationships (calls, contains, inherits/impls, imports).

use crate::{LanguageParser, ParseError, ParseResult};
use ckc_ir::{EdgeKind, IrEdge, IrNode, NodeKind, SourceLocation, SymbolId, Visibility};
use std::path::Path;
use tree_sitter::Parser;

pub struct RustParser {
    language: tree_sitter::Language,
}

impl RustParser {
    pub fn new() -> Self {
        Self {
            language: tree_sitter_rust::LANGUAGE.into(),
        }
    }

    fn make_parser(&self) -> Result<Parser, ParseError> {
        let mut parser = Parser::new();
        parser.set_language(&self.language).map_err(|e| ParseError::TreeSitter {
            path: "<rust parser init>".into(),
            message: e.to_string(),
        })?;
        Ok(parser)
    }
}

impl Default for RustParser {
    fn default() -> Self { Self::new() }
}

impl LanguageParser for RustParser {
    fn language_name(&self) -> &str { "rust" }

    fn parse_file(&self, repo_root: &Path, file_path: &Path) -> Result<ParseResult, ParseError> {
        let source = std::fs::read_to_string(file_path).map_err(|e| ParseError::Io {
            path: file_path.display().to_string(),
            source: e,
        })?;
        let mut parser = self.make_parser()?;
        let tree = parser.parse(&source, None).ok_or_else(|| ParseError::TreeSitter {
            path: file_path.display().to_string(),
            message: "parse returned None".into(),
        })?;
        let root = tree.root_node();
        let mut result = ParseResult::default();
        let rel_path = file_path.strip_prefix(repo_root).unwrap_or(file_path).display().to_string();
        let module_path = rust_module_path(&rel_path);

        // File node
        result.nodes.push(IrNode::new(
            SymbolId::new(&rel_path, Vec::new(), &rel_path, 0),
            NodeKind::File, &rel_path,
            SourceLocation { line_start: 0, line_end: root.end_position().row as u32 + 1, col_start: 0, col_end: 0 },
        ));
        let module_id = SymbolId::new(&rel_path, module_path.clone(), &rel_path, 0);

        // Extract top-level items
        for child in root.children(&mut root.walk()) {
            match child.kind() {
                "function_item" => {
                    if let Some(node) = extract_rust_function(&child, &source, &rel_path, &module_path, &module_id) {
                        collect_rust_call_edges(&child, &source, &rel_path, &node.id, &mut result.edges);
                        result.edges.push(IrEdge::new(module_id.clone(), node.id.clone(), EdgeKind::Contains));
                        result.nodes.push(node);
                    }
                }
                "struct_item" => {
                    if let Some(node) = extract_rust_struct(&child, &source, &rel_path, &module_path, &module_id) {
                        result.edges.push(IrEdge::new(module_id.clone(), node.id.clone(), EdgeKind::Contains));
                        result.nodes.push(node);
                    }
                }
                "enum_item" => {
                    if let Some(node) = extract_rust_enum(&child, &source, &rel_path, &module_path, &module_id) {
                        result.edges.push(IrEdge::new(module_id.clone(), node.id.clone(), EdgeKind::Contains));
                        result.nodes.push(node);
                    }
                }
                "trait_item" => {
                    if let Some(node) = extract_rust_trait(&child, &source, &rel_path, &module_path, &module_id) {
                        result.edges.push(IrEdge::new(module_id.clone(), node.id.clone(), EdgeKind::Contains));
                        result.nodes.push(node);
                    }
                }
                "impl_item" => {
                    if let Some((impl_node, methods, mut method_edges)) = extract_rust_impl(&child, &source, &rel_path, &module_path) {
                        result.nodes.push(impl_node);
                        result.nodes.extend(methods);
                        result.edges.append(&mut method_edges);
                    }
                }
                "mod_item" => {
                    if let Some(node) = extract_rust_module(&child, &source, &rel_path, &module_path, &module_id) {
                        result.edges.push(IrEdge::new(module_id.clone(), node.id.clone(), EdgeKind::Contains));
                        result.nodes.push(node);
                    }
                }
                "use_declaration" => {
                    if let Ok(text) = child.utf8_text(source.as_bytes()) {
                        // Strip "use " prefix and ";" suffix
                        let cleaned = text.trim()
                            .strip_prefix("use ")
                            .unwrap_or(text)
                            .trim_end_matches(';')
                            .trim()
                            .to_string();
                        let target_id = SymbolId::new(&rel_path, Vec::new(), cleaned, 0);
                        result.edges.push(IrEdge::new(module_id.clone(), target_id, EdgeKind::Imports));
                    }
                }
                _ => {}
            }
        }
        Ok(result)
    }
}

// ── Extractors ─────────────────────────────────────────────────────────────

fn rust_module_path(rel_path: &str) -> Vec<String> {
    let path = rel_path.strip_suffix(".rs").unwrap_or(rel_path);
    path.replace('\\', "/").split('/').filter(|s| *s != "mod" && *s != "lib").map(String::from).collect()
}

fn node_loc(node: &tree_sitter::Node) -> SourceLocation {
    let s = node.start_position(); let e = node.end_position();
    SourceLocation { line_start: s.row as u32 + 1, line_end: e.row as u32 + 1, col_start: s.column as u32, col_end: e.column as u32 }
}

fn child_name<'a>(node: &tree_sitter::Node, source: &'a str) -> Option<&'a str> {
    node.child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
}

fn extract_rust_function(node: &tree_sitter::Node, source: &str, rel_path: &str, module_path: &[String], _parent: &SymbolId) -> Option<IrNode> {
    let name = child_name(node, source)?;
    let id = SymbolId::new(rel_path, module_path.to_vec(), name, 0);
    Some(IrNode { id, kind: NodeKind::Function, name: name.to_string(), location: node_loc(node), visibility: Visibility::Public, metadata: Default::default(), semantic: None, hash: 0 })
}

fn extract_rust_struct(node: &tree_sitter::Node, source: &str, rel_path: &str, module_path: &[String], _parent: &SymbolId) -> Option<IrNode> {
    let name = child_name(node, source)?;
    let id = SymbolId::new(rel_path, module_path.to_vec(), name, 0);
    Some(IrNode { id, kind: NodeKind::Struct, name: name.to_string(), location: node_loc(node), visibility: Visibility::Public, metadata: Default::default(), semantic: None, hash: 0 })
}

fn extract_rust_enum(node: &tree_sitter::Node, source: &str, rel_path: &str, module_path: &[String], _parent: &SymbolId) -> Option<IrNode> {
    let name = child_name(node, source)?;
    let id = SymbolId::new(rel_path, module_path.to_vec(), name, 0);
    Some(IrNode { id, kind: NodeKind::Enum, name: name.to_string(), location: node_loc(node), visibility: Visibility::Public, metadata: Default::default(), semantic: None, hash: 0 })
}

fn extract_rust_trait(node: &tree_sitter::Node, source: &str, rel_path: &str, module_path: &[String], _parent: &SymbolId) -> Option<IrNode> {
    let name = child_name(node, source)?;
    let id = SymbolId::new(rel_path, module_path.to_vec(), name, 0);
    Some(IrNode { id, kind: NodeKind::Trait, name: name.to_string(), location: node_loc(node), visibility: Visibility::Public, metadata: Default::default(), semantic: None, hash: 0 })
}

fn extract_rust_impl(node: &tree_sitter::Node, source: &str, rel_path: &str, module_path: &[String]) -> Option<(IrNode, Vec<IrNode>, Vec<IrEdge>)> {
    // Get the trait/type name being implemented
    let type_name = node.child_by_field_name("type")
        .or_else(|| node.child_by_field_name("trait"))
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .unwrap_or("impl");
    let impl_name = format!("impl {}", type_name);
    let id = SymbolId::new(rel_path, module_path.to_vec(), &impl_name, 0);
    let impl_node = IrNode { id: id.clone(), kind: NodeKind::TraitImpl, name: impl_name.clone(), location: node_loc(node), visibility: Visibility::Public, metadata: Default::default(), semantic: None, hash: 0 };

    let mut methods = Vec::new();
    let mut edges = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        for child in body.children(&mut body.walk()) {
            if child.kind() == "function_item" {
                if let Some(mut m) = extract_rust_function(&child, source, rel_path, module_path, &id) {
                    m.kind = NodeKind::Method;
                    collect_rust_call_edges(&child, source, rel_path, &m.id, &mut edges);
                    methods.push(m);
                }
            }
        }
    }
    Some((impl_node, methods, edges))
}

fn extract_rust_module(node: &tree_sitter::Node, source: &str, rel_path: &str, module_path: &[String], _parent: &SymbolId) -> Option<IrNode> {
    let name = child_name(node, source)?;
    let mut mp = module_path.to_vec();
    mp.push(name.to_string());
    let id = SymbolId::new(rel_path, mp, name, 0);
    Some(IrNode { id, kind: NodeKind::Module, name: name.to_string(), location: node_loc(node), visibility: Visibility::Public, metadata: Default::default(), semantic: None, hash: 0 })
}

/// Recursively collect call edges from Rust call expressions.
fn collect_rust_call_edges(
    node: &tree_sitter::Node,
    source: &str,
    rel_path: &str,
    caller_id: &SymbolId,
    edges: &mut Vec<IrEdge>,
) {
    if node.kind() == "call_expression" {
        if let Some(func) = node.child_by_field_name("function") {
            let callee_name = match func.kind() {
                "identifier" | "scoped_identifier" | "field_expression" => {
                    func.utf8_text(source.as_bytes()).ok().map(|s| s.to_string())
                }
                _ => None,
            };
            if let Some(ref name) = callee_name {
                let mut edge = IrEdge::new(
                    caller_id.clone(),
                    SymbolId::new(rel_path, Vec::new(), name, 0),
                    EdgeKind::Calls,
                );
                edge.metadata.insert(
                    "target_name".into(),
                    serde_json::Value::String(name.clone()),
                );
                edges.push(edge);
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_rust_call_edges(&child, source, rel_path, caller_id, edges);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_rust(source: &str) -> ParseResult {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(1000);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let parser = RustParser::new();
        let tmp = std::env::temp_dir();
        let file = tmp.join(format!("_ckc_rust_test_{}.rs", id));
        std::fs::write(&file, source).unwrap();
        let result = parser.parse_file(&tmp, &file).unwrap();
        let _ = std::fs::remove_file(&file);
        result
    }

    #[test]
    fn parse_rust_function() {
        let result = parse_rust("fn hello() { println!(\"hi\"); }");
        let funcs: Vec<_> = result.nodes.iter().filter(|n| n.kind == NodeKind::Function).collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "hello");
    }

    #[test]
    fn parse_rust_struct() {
        let result = parse_rust("pub struct Point { x: i32, y: i32 }");
        let structs: Vec<_> = result.nodes.iter().filter(|n| n.kind == NodeKind::Struct).collect();
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, "Point");
    }

    #[test]
    fn parse_rust_enum() {
        let result = parse_rust("enum Color { Red, Green, Blue }");
        let enums: Vec<_> = result.nodes.iter().filter(|n| n.kind == NodeKind::Enum).collect();
        assert_eq!(enums.len(), 1);
        assert_eq!(enums[0].name, "Color");
    }

    #[test]
    fn parse_rust_trait_and_impl() {
        let result = parse_rust("trait Draw { fn draw(&self); } impl Draw for Circle { fn draw(&self) {} }");
        let traits: Vec<_> = result.nodes.iter().filter(|n| n.kind == NodeKind::Trait).collect();
        let impls: Vec<_> = result.nodes.iter().filter(|n| n.kind == NodeKind::TraitImpl).collect();
        assert_eq!(traits.len(), 1);
        assert_eq!(impls.len(), 1);
    }

    #[test]
    fn parse_rust_use() {
        let result = parse_rust("use std::collections::HashMap;");
        let imports: Vec<_> = result.edges.iter().filter(|e| e.kind == EdgeKind::Imports).collect();
        assert!(imports.len() >= 1);
    }
}
