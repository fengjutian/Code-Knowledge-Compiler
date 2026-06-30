//! Cross-file symbol resolution via import graph.
//!
//! After all files are parsed, the [`SymbolResolver`] builds:
//!   1. A global symbol index (name → set of defining files)
//!   2. A per-file import map (imported name → resolved file path)
//!
//! It then rewrites call edges that reference symbols from other files,
//! replacing placeholder SymbolIds with correct target keys.

use ckc_ir::{EdgeKind, IrEdge, IrNode, SymbolId};
use std::collections::{HashMap, HashSet};

/// Resolves cross-file symbol references using import information.
pub struct SymbolResolver {
    /// symbol_name → list of (file_path, SymbolId) where it's defined
    symbol_index: HashMap<String, Vec<(String, SymbolId)>>,
}

impl SymbolResolver {
    /// Build a resolver from all parsed nodes.
    pub fn new(nodes: &[IrNode]) -> Self {
        let mut symbol_index: HashMap<String, Vec<(String, SymbolId)>> = HashMap::new();

        for node in nodes {
            // Only index definable symbols (skip File nodes, variables, etc.)
            if matches!(
                node.kind,
                ckc_ir::NodeKind::Function
                    | ckc_ir::NodeKind::Method
                    | ckc_ir::NodeKind::Class
                    | ckc_ir::NodeKind::Module
            ) {
                symbol_index
                    .entry(node.name.clone())
                    .or_default()
                    .push((node.id.file_path.clone(), node.id.clone()));
            }
        }

        Self { symbol_index }
    }

    /// Build a file path → import resolution map from parsed import edges.
    ///
    /// Maps `import X` / `from X import Y` to resolved file paths.
    fn build_import_map(
        &self,
        edges: &[IrEdge],
        all_file_paths: &HashSet<String>,
    ) -> HashMap<String, HashMap<String, String>> {
        // file_path → (imported_name → resolved_file_path)
        let mut file_imports: HashMap<String, HashMap<String, String>> = HashMap::new();

        for edge in edges {
            if edge.kind != EdgeKind::Imports {
                continue;
            }

            let source_file = &edge.source_id.file_path;
            let imported_name = &edge.target_id.name;

            // For `from X import Y`, use the module path X to resolve
            let module_name = edge.metadata.get("import_module").and_then(|v| v.as_str());
            let resolved = if let Some(module) = module_name {
                // Resolve using the module path, then map imported_name to that file
                self.resolve_import_to_file(module, all_file_paths)
            } else {
                self.resolve_import_to_file(imported_name, all_file_paths)
            };

            if let Some(resolved_file) = resolved {
                file_imports
                    .entry(source_file.clone())
                    .or_default()
                    .insert(imported_name.clone(), resolved_file);
            }
        }

        file_imports
    }

    /// Resolve a Python import name to a file path.
    ///
    /// e.g., "models" → "models.py", "pkg.payments" → "pkg/payments.py"
    fn resolve_import_to_file(
        &self,
        import_name: &str,
        all_file_paths: &HashSet<String>,
    ) -> Option<String> {
        let sep = std::path::MAIN_SEPARATOR_STR;

        // Try as a direct module: "models" → "models.py"
        let direct = format!("{}.py", import_name);
        if all_file_paths.contains(&direct) {
            return Some(direct);
        }

        // Try as a package: "pkg.payments" → "pkg/payments.py" (normalized for OS)
        let dotted = format!("{}.py", import_name.replace('.', sep));
        if all_file_paths.contains(&dotted) {
            return Some(dotted);
        }

        // Try as package __init__: "pkg" → "pkg/__init__.py"
        let init = format!("{}{}__init__.py", import_name.replace('.', sep), sep);
        if all_file_paths.contains(&init) {
            return Some(init);
        }

        None
    }

    /// Resolve call edges to point to correct cross-file symbols.
    ///
    /// For each Calls edge with a `target_name`:
    ///   1. Try to find the symbol in the same file
    ///   2. Handle `self.method` / `obj.method` patterns: strip object prefix
    ///   3. If not found locally, check imported files
    ///   4. If found in an imported file, rewrite the edge
    pub fn resolve_calls(
        &self,
        edges: &mut Vec<IrEdge>,
        all_file_paths: &HashSet<String>,
    ) -> usize {
        let file_imports = self.build_import_map(edges, all_file_paths);
        let mut resolved_count = 0;

        for edge in edges.iter_mut() {
            if edge.kind != EdgeKind::Calls {
                continue;
            }

            // Get the target_name from metadata
            let raw_name = match edge.metadata.get("target_name").and_then(|v| v.as_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            let caller_file = &edge.source_id.file_path;

            // Try multiple name variants for attribute calls (self.add → try "self.add", then "add")
            let name_variants = if raw_name.contains('.') {
                let last = raw_name.split('.').last().unwrap_or(&raw_name).to_string();
                vec![raw_name.clone(), last]
            } else {
                vec![raw_name.clone()]
            };

            for target_name in &name_variants {
                // Step 1: Check if target exists in same file
                if self.symbol_exists_in_file(target_name, caller_file) {
                    edge.metadata.insert(
                        "target_name".into(),
                        serde_json::Value::String(target_name.clone()),
                    );
                    break;
                }

                // Step 2: Check imported files
                let mut found_in_import = false;
                if let Some(candidates) = self.symbol_index.get(target_name) {
                    if let Some(imports) = file_imports.get(caller_file) {
                        for (candidate_file, candidate_id) in candidates {
                            if candidate_file == caller_file {
                                continue;
                            }
                            if imports.values().any(|f| f == candidate_file) {
                                edge.target_id = candidate_id.clone();
                                edge.metadata.insert(
                                    "target_name".into(),
                                    serde_json::Value::String(target_name.clone()),
                                );
                                resolved_count += 1;
                                found_in_import = true;
                                break;
                            }
                        }
                    }
                }
                if found_in_import {
                    break;
                }
            }
        }

        resolved_count
    }

    fn symbol_exists_in_file(&self, name: &str, file_path: &str) -> bool {
        self.symbol_index
            .get(name)
            .map(|candidates| candidates.iter().any(|(f, _)| f == file_path))
            .unwrap_or(false)
    }
}

/// Build the set of all file paths from nodes.
pub fn collect_file_paths(nodes: &[IrNode]) -> HashSet<String> {
    nodes
        .iter()
        .filter(|n| n.kind == ckc_ir::NodeKind::File)
        .map(|n| n.id.file_path.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ckc_ir::{EdgeKind, IrEdge, IrNode, NodeKind, SourceLocation, SymbolId, Visibility};

    fn make_node(name: &str, kind: NodeKind, file: &str) -> IrNode {
        let id = SymbolId::new(file, Vec::new(), name, 0);
        IrNode {
            id,
            kind,
            name: name.to_string(),
            location: SourceLocation {
                line_start: 1,
                line_end: 1,
                col_start: 0,
                col_end: 0,
            },
            visibility: Visibility::Public,
            metadata: Default::default(),
            semantic: None,
            hash: 0,
        }
    }

    #[test]
    fn resolve_simple_import() {
        let nodes = vec![
            make_node("callee", NodeKind::Function, "lib.py"),
            make_node("caller", NodeKind::Function, "main.py"),
        ];
        let resolver = SymbolResolver::new(&nodes);

        let mut edges = vec![
            // import edge: main.py imports lib
            IrEdge::new(
                SymbolId::new("main.py", Vec::new(), "main.py", 0),
                SymbolId::new("main.py", Vec::new(), "lib", 0),
                EdgeKind::Imports,
            ),
        ];
        // Call edge: caller → callee (target_name = "callee")
        let mut call_edge = IrEdge::new(
            SymbolId::new("main.py", Vec::new(), "caller", 0),
            SymbolId::new("main.py", Vec::new(), "callee", 0),
            EdgeKind::Calls,
        );
        call_edge
            .metadata
            .insert("target_name".into(), serde_json::Value::String("callee".into()));
        edges.push(call_edge);

        // Add File nodes for collect_file_paths
        let mut all_nodes = nodes.clone();
        all_nodes.push(make_node("lib.py", NodeKind::File, "lib.py"));
        all_nodes.push(make_node("main.py", NodeKind::File, "main.py"));
        let file_paths = collect_file_paths(&all_nodes);

        let count = resolver.resolve_calls(&mut edges, &file_paths);
        assert_eq!(count, 1);
        // The call edge should now point to lib.py's callee
        let call = edges.last().unwrap();
        assert_eq!(call.target_id.file_path, "lib.py");
    }

    #[test]
    fn no_resolve_when_same_file() {
        let nodes = vec![
            make_node("helper", NodeKind::Function, "mod.py"),
            make_node("main_func", NodeKind::Function, "mod.py"),
        ];
        let resolver = SymbolResolver::new(&nodes);

        let mut edges = Vec::new();
        let mut call_edge = IrEdge::new(
            SymbolId::new("mod.py", Vec::new(), "main_func", 0),
            SymbolId::new("mod.py", Vec::new(), "helper", 0),
            EdgeKind::Calls,
        );
        call_edge
            .metadata
            .insert("target_name".into(), serde_json::Value::String("helper".into()));
        edges.push(call_edge);

        let mut all_nodes = nodes.clone();
        all_nodes.push(make_node("mod.py", NodeKind::File, "mod.py"));
        let file_paths = collect_file_paths(&all_nodes);

        let count = resolver.resolve_calls(&mut edges, &file_paths);
        assert_eq!(count, 0); // Same file, no resolution needed
    }
}
