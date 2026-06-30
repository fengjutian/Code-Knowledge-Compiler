//! Language parser abstraction and Python implementation.
//!
//! The [`LanguageParser`] trait defines the interface for extracting
//! Knowledge IR nodes and edges from source code.

mod rust_parser;
pub use rust_parser::RustParser;

use ckc_ir::{
    EdgeKind, IrEdge, IrNode, NodeKind, SemanticInfo, SourceLocation, SymbolId, Visibility,
};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;
use tree_sitter::Parser;

// ── Error ──────────────────────────────────────────────────────────────────

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("I/O error reading {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("tree-sitter parse error in {path}: {message}")]
    TreeSitter { path: String, message: String },
    #[error("unsupported language: {0}")]
    UnsupportedLanguage(String),
}

// ── Trait ──────────────────────────────────────────────────────────────────

/// Extract Knowledge IR nodes and edges from source code.
pub trait LanguageParser {
    fn language_name(&self) -> &str;
    fn parse_file(&self, repo_root: &Path, file_path: &Path) -> Result<ParseResult, ParseError>;
}

#[derive(Debug, Default)]
pub struct ParseResult {
    pub nodes: Vec<IrNode>,
    pub edges: Vec<IrEdge>,
    pub warnings: Vec<String>,
}

// ── Python Parser ──────────────────────────────────────────────────────────

pub struct PythonParser {
    language: tree_sitter::Language,
}

impl PythonParser {
    pub fn new() -> Self {
        Self {
            language: tree_sitter_python::LANGUAGE.into(),
        }
    }

    fn make_parser(&self) -> Result<Parser, ParseError> {
        let mut parser = Parser::new();
        parser.set_language(&self.language).map_err(|e| {
            ParseError::TreeSitter {
                path: "<parser init>".into(),
                message: e.to_string(),
            }
        })?;
        Ok(parser)
    }
}

impl Default for PythonParser {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageParser for PythonParser {
    fn language_name(&self) -> &str {
        "python"
    }

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

        let rel_path = file_path
            .strip_prefix(repo_root)
            .unwrap_or(file_path)
            .display()
            .to_string();

        let module_path = file_path_to_module_path(&rel_path);

        // File node
        let file_id = SymbolId::new(&rel_path, Vec::new(), &rel_path, 0);
        result.nodes.push(IrNode::new(
            file_id.clone(),
            NodeKind::File,
            &rel_path,
            SourceLocation {
                line_start: 0,
                line_end: root.end_position().row as u32 + 1,
                col_start: 0,
                col_end: 0,
            },
        ));

        let module_id = SymbolId::new(&rel_path, module_path.clone(), &rel_path, 0);

        // Process top-level children, handling nested decorated_definitions
        process_top_level_items(
            &root, &source, &rel_path, &module_path, &module_id, &mut result,
        );

        Ok(result)
    }
}

// ── Recursive Item Processing ──────────────────────────────────────────────

/// Process children of a module/class/function body, handling decorated and
/// nested definitions recursively.
fn process_body_items(
    parent_node: &tree_sitter::Node,
    source: &str,
    rel_path: &str,
    module_path: &[String],
    parent_id: &SymbolId,
    result: &mut ParseResult,
) {
    for child in parent_node.children(&mut parent_node.walk()) {
        match child.kind() {
            "decorated_definition" => {
                let decorators = collect_all_decorators(&child, source);
                if let Some(definition) = find_definition(&child) {
                    match definition.kind() {
                        "function_definition" => {
                            if let Some((node, call_edges, nested_nodes, nested_edges)) = extract_function(
                                &definition, source, rel_path, module_path, parent_id,
                                Some(&decorators),
                            ) {
                                result.edges.push(IrEdge::new(
                                    parent_id.clone(), node.id.clone(), EdgeKind::Contains,
                                ));
                                result.edges.extend(call_edges);
                                result.nodes.push(node);
                                result.nodes.extend(nested_nodes);
                                result.edges.extend(nested_edges);
                            }
                        }
                        "class_definition" => {
                            if let Some((node, inh, methods, mcalls)) = extract_class_members(
                                &definition, source, rel_path, module_path,
                                Some(&decorators),
                            ) {
                                result.edges.push(IrEdge::new(
                                    parent_id.clone(), node.id.clone(), EdgeKind::Contains,
                                ));
                                result.edges.extend(inh);
                                result.nodes.push(node);
                                result.nodes.extend(methods);
                                result.edges.extend(mcalls);
                            }
                        }
                        _ => {}
                    }
                }
            }
            "function_definition" => {
                if let Some((node, call_edges, nested_nodes, nested_edges)) = extract_function(
                    &child, source, rel_path, module_path, parent_id, None,
                ) {
                    result.edges.push(IrEdge::new(
                        parent_id.clone(), node.id.clone(), EdgeKind::Contains,
                    ));
                    result.edges.extend(call_edges);
                    result.nodes.push(node);
                    result.nodes.extend(nested_nodes);
                    result.edges.extend(nested_edges);
                }
            }
            "class_definition" => {
                if let Some((node, inh, methods, mcalls)) = extract_class_members(
                    &child, source, rel_path, module_path, None,
                ) {
                    result.edges.push(IrEdge::new(
                        parent_id.clone(), node.id.clone(), EdgeKind::Contains,
                    ));
                    result.edges.extend(inh);
                    result.nodes.push(node);
                    result.nodes.extend(methods);
                    result.edges.extend(mcalls);
                }
            }
            "import_statement" | "import_from_statement" => {
                for edge in extract_imports(&child, source, rel_path, parent_id) {
                    result.edges.push(edge);
                }
            }
            // Structural patterns: tag parent function with metadata
            "try_statement" => {
                result.warnings.push(format!(
                    "{}:{}: try-except detected in scope of {}",
                    rel_path,
                    child.start_position().row + 1,
                    parent_id.name
                ));
            }
            "lambda" => {
                result.warnings.push(format!(
                    "{}:{}: lambda detected in scope of {}",
                    rel_path,
                    child.start_position().row + 1,
                    parent_id.name
                ));
            }
            "yield" => {
                result.warnings.push(format!(
                    "{}:{}: yield detected in scope of {} (generator)",
                    rel_path,
                    child.start_position().row + 1,
                    parent_id.name
                ));
            }
            "with_statement" => {
                result.warnings.push(format!(
                    "{}:{}: with-statement context manager in scope of {}",
                    rel_path,
                    child.start_position().row + 1,
                    parent_id.name
                ));
            }
            "expression_statement" => {
                // Module/class-level variables. Inside functions we skip these
                // to avoid extracting local variables.
                if parent_id.name.contains(".py")
                    || matches!(
                        parent_node.kind(),
                        "class_definition" | "block"
                    )
                {
                    if let Some(var_node) =
                        extract_module_variable(&child, source, rel_path, module_path)
                    {
                        result.edges.push(IrEdge::new(
                            parent_id.clone(), var_node.id.clone(), EdgeKind::Contains,
                        ));
                        result.nodes.push(var_node);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Top-level processing entry point (delegates to process_body_items).
fn process_top_level_items(
    root: &tree_sitter::Node,
    source: &str,
    rel_path: &str,
    module_path: &[String],
    module_id: &SymbolId,
    result: &mut ParseResult,
) {
    process_body_items(root, source, rel_path, module_path, module_id, result);
}

/// Collect all decorators from a (possibly nested) decorated_definition.
///
/// tree-sitter nests stacked decorators:
///   `@a\n@b\ndef foo()` →
///   `decorated_definition { decorator:@a, definition: decorated_definition { decorator:@b, definition: function_definition } }`
///
/// This function walks up to 3 levels deep (handles 99% of real-world cases).
fn collect_all_decorators(node: &tree_sitter::Node, source: &str) -> Vec<String> {
    let mut all = extract_decorators(node, source);

    // Check for nested decorated_definition
    if let Some(child) = node.child_by_field_name("definition") {
        if child.kind() == "decorated_definition" {
            all.extend(extract_decorators(&child, source));
            // One more level
            if let Some(grandchild) = child.child_by_field_name("definition") {
                if grandchild.kind() == "decorated_definition" {
                    all.extend(extract_decorators(&grandchild, source));
                }
            }
        }
    }
    all
}

/// Find the innermost non-decorated definition node.
/// Returns the function_definition or class_definition after unwrapping decorators.
fn find_definition<'a>(node: &'a tree_sitter::Node) -> Option<tree_sitter::Node<'a>> {
    let def = node.child_by_field_name("definition")?;
    if def.kind() == "decorated_definition" {
        // One more level
        let inner = def.child_by_field_name("definition")?;
        if inner.kind() == "decorated_definition" {
            inner.child_by_field_name("definition")
        } else {
            Some(inner)
        }
    } else {
        Some(def)
    }
}

/// Extract decorator names from a decorated_definition node.
fn extract_decorators(node: &tree_sitter::Node, source: &str) -> Vec<String> {
    let mut decorators = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "decorator" {
            if let Some(name) = extract_decorator_name(&child, source) {
                decorators.push(name);
            }
        }
    }
    decorators
}

/// Extract the logical name from a decorator node (skip @ sign, handle call/attribute/identifier).
fn extract_decorator_name(decorator: &tree_sitter::Node, source: &str) -> Option<String> {
    for child in decorator.children(&mut decorator.walk()) {
        match child.kind() {
            "@" => continue,
            "identifier" => return child.utf8_text(source.as_bytes()).ok().map(|s| s.to_string()),
            "attribute" => return child.utf8_text(source.as_bytes()).ok().map(|s| s.to_string()),
            "call" => {
                // @decorator(args) — extract the callable name
                return extract_decorator_name(&child, source);
            }
            _ => continue,
        }
    }
    None
}

/// Apply decorator semantics to a function/method node.
fn apply_decorators(node: &mut IrNode, decorators: &[String]) {
    for d in decorators {
        match d.as_str() {
            "staticmethod" | "classmethod" => {
                // Mark in metadata
                node.metadata
                    .insert("decorator".into(), serde_json::Value::String(d.clone()));
            }
            "property" => {
                node.metadata
                    .insert("property".into(), serde_json::Value::Bool(true));
            }
            "abstractmethod" => {
                node.metadata
                    .insert("abstract".into(), serde_json::Value::Bool(true));
            }
            _ => {
                // Custom decorator — record it
                let decorators = node
                    .metadata
                    .entry("decorators".into())
                    .or_insert_with(|| serde_json::Value::Array(Vec::new()));
                if let Some(arr) = decorators.as_array_mut() {
                    arr.push(serde_json::Value::String(d.clone()));
                }
            }
        }
    }
}

// ── Docstring Extraction ───────────────────────────────────────────────────

/// Extract a docstring from the body of a function or class.
fn extract_docstring(body: &tree_sitter::Node, source: &str) -> Option<String> {
    // The docstring is the first expression_statement containing a string
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "expression_statement" {
            for expr_child in child.children(&mut child.walk()) {
                if expr_child.kind() == "string" {
                    // Get raw text (includes quotes)
                    let raw = expr_child.utf8_text(source.as_bytes()).ok()?;
                    // Handle triple-quoted strings
                    let cleaned = raw
                        .trim_start_matches("\"\"\"")
                        .trim_start_matches("'''")
                        .trim_end_matches("\"\"\"")
                        .trim_end_matches("'''")
                        .trim_start_matches('"')
                        .trim_start_matches('\'')
                        .trim_end_matches('"')
                        .trim_end_matches('\'')
                        .trim();
                    if !cleaned.is_empty() {
                        // Take first line only as purpose
                        let first_line = cleaned.lines().next().unwrap_or("").trim();
                        if !first_line.is_empty() {
                            return Some(first_line.to_string());
                        }
                    }
                }
            }
            // Only check the first statement
            break;
        }
    }
    None
}

// ── Type Annotation Extraction ─────────────────────────────────────────────

/// Extract type annotation from a parameter or return type node.
fn type_annotation_text(node: &tree_sitter::Node, source: &str) -> Option<String> {
    node.child_by_field_name("type")
        .or_else(|| {
            // For return type in function_definition
            node.child_by_field_name("return_type")
        })
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
}

/// Extract parameter list with type annotations.
fn extract_parameters(
    func_node: &tree_sitter::Node,
    source: &str,
) -> Vec<HashMap<String, String>> {
    let mut params = Vec::new();
    if let Some(parameters) = func_node.child_by_field_name("parameters") {
        for child in parameters.children(&mut parameters.walk()) {
            if child.kind() == "identifier" {
                // Bare parameter name
                let name = child.utf8_text(source.as_bytes()).unwrap_or("").to_string();
                if !name.is_empty() && name != "self" && name != "cls" {
                    params.push({
                        let mut m = HashMap::new();
                        m.insert("name".into(), name);
                        m
                    });
                }
            } else if child.kind() == "typed_parameter" {
                if let Some(name_node) = find_child_of_kind(&child, "identifier") {
                    let name = name_node.utf8_text(source.as_bytes()).unwrap_or("").to_string();
                    if !name.is_empty() && name != "self" && name != "cls" {
                        let type_ann = child
                            .child_by_field_name("type")
                            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                            .map(|s| s.to_string());
                        let mut m = HashMap::new();
                        m.insert("name".into(), name);
                        if let Some(t) = type_ann {
                            m.insert("type".into(), t);
                        }
                        params.push(m);
                    }
                }
            } else if child.kind() == "default_parameter" {
                if let Some(name_node) = find_child_of_kind(&child, "identifier") {
                    let name = name_node.utf8_text(source.as_bytes()).unwrap_or("").to_string();
                    if !name.is_empty() && name != "self" && name != "cls" {
                        let type_ann = child
                            .child_by_field_name("type")
                            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                            .map(|s| s.to_string());
                        let mut m = HashMap::new();
                        m.insert("name".into(), name);
                        if let Some(t) = type_ann {
                            m.insert("type".into(), t);
                        }
                        m.insert("has_default".into(), "true".into());
                        params.push(m);
                    }
                }
            }
        }
    }
    params
}

/// Compute a signature hash from function name + parameter types.
fn compute_signature_hash(name: &str, params: &[HashMap<String, String>], return_type: Option<&str>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    for p in params {
        if let Some(n) = p.get("name") {
            n.hash(&mut hasher);
        }
        if let Some(t) = p.get("type") {
            t.hash(&mut hasher);
        }
    }
    if let Some(rt) = return_type {
        rt.hash(&mut hasher);
    }
    hasher.finish()
}

// ── Variable Extraction ────────────────────────────────────────────────────

/// Extract a module-level variable assignment.
fn extract_module_variable(
    node: &tree_sitter::Node,
    source: &str,
    rel_path: &str,
    module_path: &[String],
) -> Option<IrNode> {
    // expression_statement → assignment / annotated_assignment
    for child in node.children(&mut node.walk()) {
        match child.kind() {
            "assignment" => {
                if let Some(lhs) = child.child_by_field_name("left") {
                    if let Ok(name) = lhs.utf8_text(source.as_bytes()) {
                        let id = SymbolId::new(rel_path, module_path.to_vec(), name, 0);
                        let mut node = IrNode::new(
                            id.clone(),
                            NodeKind::Variable,
                            name,
                            node_location(node),
                        );
                        node.visibility = Visibility::Private;
                        return Some(node);
                    }
                }
            }
            "annotated_assignment" => {
                if let Some(lhs) = child.child_by_field_name("left") {
                    if let Ok(name) = lhs.utf8_text(source.as_bytes()) {
                        let id = SymbolId::new(rel_path, module_path.to_vec(), name, 0);
                        let mut var_node = IrNode::new(
                            id.clone(),
                            NodeKind::Variable,
                            name,
                            node_location(node),
                        );
                        var_node.visibility = Visibility::Private;
                        if let Some(type_ann) = child
                            .child_by_field_name("type")
                            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                        {
                            var_node
                                .metadata
                                .insert("type".into(), serde_json::Value::String(type_ann.to_string()));
                        }
                        return Some(var_node);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

// ── Function / Method Extraction ───────────────────────────────────────────

/// Extract a function/method node with nested definitions.
fn extract_function(
    node: &tree_sitter::Node,
    source: &str,
    rel_path: &str,
    module_path: &[String],
    _parent_id: &SymbolId,
    decorators: Option<&[String]>,
) -> Option<(IrNode, Vec<IrEdge>, Vec<IrNode>, Vec<IrEdge>)> {
    let name = get_child_text(node, source, "identifier")?;
    let location = node_location(node);

    let is_async = has_child_of_kind(node, "async");
    // Always Function here — callers override to Method for class methods
    let kind = NodeKind::Function;

    // Extract parameters and return type for signature hash
    let params = extract_parameters(node, source);
    let return_type = type_annotation_text(node, source);
    let signature_hash = compute_signature_hash(name, &params, return_type.as_deref());

    let id = SymbolId::new(rel_path, module_path.to_vec(), name, signature_hash);

    let mut ir_node = IrNode {
        id: id.clone(),
        kind,
        name: name.to_string(),
        location,
        visibility: Visibility::Public,
        metadata: Default::default(),
        semantic: None,
        hash: 0,
    };

    // Type annotations → metadata
    if !params.is_empty() {
        let json_params: Vec<serde_json::Value> = params
            .iter()
            .map(|p| {
                let mut m = serde_json::Map::new();
                for (k, v) in p {
                    m.insert(k.clone(), serde_json::Value::String(v.clone()));
                }
                serde_json::Value::Object(m)
            })
            .collect();
        ir_node
            .metadata
            .insert("parameters".into(), serde_json::Value::Array(json_params));
    }
    if let Some(rt) = &return_type {
        ir_node
            .metadata
            .insert("return_type".into(), serde_json::Value::String(rt.clone()));
    }
    if is_async {
        ir_node
            .metadata
            .insert("async".into(), serde_json::Value::Bool(true));
    }

    // Apply decorators
    if let Some(decs) = decorators {
        apply_decorators(&mut ir_node, decs);
    }

    // Extract docstring → purpose
    if let Some(body) = node.child_by_field_name("body") {
        if let Some(doc) = extract_docstring(&body, source) {
            ir_node.semantic = Some(SemanticInfo {
                purpose: Some(doc),
                ..Default::default()
            });
        }
    }

    let call_edges = extract_calls(node, source, rel_path, &id);

    // Process nested definitions inside the function body
    let mut nested_nodes = Vec::new();
    let mut nested_edges = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        process_body_items(
            &body, source, rel_path, module_path, &id,
            &mut ParseResult { nodes: nested_nodes.clone(), edges: nested_edges.clone(), warnings: Vec::new() },
        );
        // We'll rebuild this properly
        nested_nodes.clear();
        nested_edges.clear();
        let mut nested_result = ParseResult::default();
        process_body_items(
            &body, source, rel_path, module_path, &id, &mut nested_result,
        );
        for nested_node in &nested_result.nodes {
            nested_edges.push(IrEdge::new(
                id.clone(), nested_node.id.clone(), EdgeKind::Contains,
            ));
        }
        nested_nodes = nested_result.nodes;
        nested_edges.extend(nested_result.edges);
    }

    Some((ir_node, call_edges, nested_nodes, nested_edges))
}

// ── Class Extraction ───────────────────────────────────────────────────────

/// Extract a class node with methods, inheritance, and decorators.
fn extract_class_members(
    node: &tree_sitter::Node,
    source: &str,
    rel_path: &str,
    module_path: &[String],
    decorators: Option<&[String]>,
) -> Option<(IrNode, Vec<IrEdge>, Vec<IrNode>, Vec<IrEdge>)> {
    let name = get_child_text(node, source, "identifier")?;
    let location = node_location(node);

    let id = SymbolId::new(rel_path, module_path.to_vec(), name, 0);
    let mut ir_node = IrNode {
        id: id.clone(),
        kind: NodeKind::Class,
        name: name.to_string(),
        location,
        visibility: Visibility::Public,
        metadata: Default::default(),
        semantic: None,
        hash: 0,
    };

    // Decorators on class
    if let Some(decs) = decorators {
        apply_decorators(&mut ir_node, decs);
    }

    // Inheritance edges
    let mut inherit_edges = Vec::new();
    if let Some(superclasses) = node.child_by_field_name("superclasses") {
        for child in superclasses.children(&mut superclasses.walk()) {
            if child.kind() == "identifier" {
                if let Ok(name_text) = child.utf8_text(source.as_bytes()) {
                    let super_id = SymbolId::new(rel_path, module_path.to_vec(), name_text, 0);
                    inherit_edges.push(IrEdge::new(id.clone(), super_id, EdgeKind::Inherits));
                }
            }
        }
    }

    // Class-level docstring
    if let Some(body) = node.child_by_field_name("body") {
        if let Some(doc) = extract_docstring(&body, source) {
            ir_node.semantic = Some(SemanticInfo {
                purpose: Some(doc),
                ..Default::default()
            });
        }
    }

    // Process class body: methods + class variables
    let mut methods = Vec::new();
    let mut method_call_edges = Vec::new();
    let class_module_path: Vec<String> = {
        let mut mp = module_path.to_vec();
        mp.push(name.to_string());
        mp
    };

    if let Some(body) = node.child_by_field_name("body") {
        for child in body.children(&mut body.walk()) {
            match child.kind() {
                "function_definition" => {
                    // A method inside the class (no decorators)
                    if let Some((method_node, calls, nested_nodes, nested_edges)) = extract_function(
                        &child, source, rel_path, &class_module_path, &id, None,
                    ) {
                        let mut mn = method_node;
                        mn.kind = NodeKind::Method;
                        method_call_edges.push(IrEdge::new(
                            id.clone(), mn.id.clone(), EdgeKind::Contains,
                        ));
                        method_call_edges.extend(calls);
                        methods.push(mn);
                        methods.extend(nested_nodes);
                        method_call_edges.extend(nested_edges);
                    }
                }
                "class_definition" => {
                    // Nested class inside a class (no decorators)
                    if let Some((node, inh, inner_methods, mcalls)) = extract_class_members(
                        &child, source, rel_path, &class_module_path, None,
                    ) {
                        method_call_edges.push(IrEdge::new(
                            id.clone(), node.id.clone(), EdgeKind::Contains,
                        ));
                        method_call_edges.extend(inh);
                        methods.push(node);
                        methods.extend(inner_methods);
                        method_call_edges.extend(mcalls);
                    }
                }
                "decorated_definition" => {
                    let method_decorators = collect_all_decorators(&child, source);
                    if let Some(def) = find_definition(&child) {
                    if def.kind() == "function_definition" {
                        if let Some((method_node, calls, nested_nodes, nested_edges)) = extract_function(
                            &def, source, rel_path, &class_module_path, &id,
                            Some(&method_decorators),
                        ) {
                            let mut mn = method_node;
                            mn.kind = NodeKind::Method;
                            method_call_edges.push(IrEdge::new(
                                id.clone(), mn.id.clone(), EdgeKind::Contains,
                            ));
                            method_call_edges.extend(calls);
                            methods.push(mn);
                            methods.extend(nested_nodes);
                            method_call_edges.extend(nested_edges);
                        }
                    } else if def.kind() == "class_definition" {
                        // Nested class inside a class with decorators
                        if let Some((node, inh, inner_methods, mcalls)) = extract_class_members(
                            &def, source, rel_path, &class_module_path,
                            Some(&method_decorators),
                        ) {
                            method_call_edges.push(IrEdge::new(
                                id.clone(), node.id.clone(), EdgeKind::Contains,
                            ));
                            method_call_edges.extend(inh);
                            methods.push(node);
                            methods.extend(inner_methods);
                            method_call_edges.extend(mcalls);
                        }
                    }
                    }
                }
                "expression_statement" => {
                    // Class-level variable
                    if let Some(var_node) = extract_class_variable(
                        &child, source, rel_path, &class_module_path, &id,
                    ) {
                        method_call_edges.push(IrEdge::new(
                            id.clone(),
                            var_node.id.clone(),
                            EdgeKind::Contains,
                        ));
                        methods.push(var_node);
                    }
                }
                _ => {}
            }
        }
    }

    Some((ir_node, inherit_edges, methods, method_call_edges))
}

/// Extract a class-level variable (class attribute).
fn extract_class_variable(
    node: &tree_sitter::Node,
    source: &str,
    rel_path: &str,
    class_module_path: &[String],
    _class_id: &SymbolId,
) -> Option<IrNode> {
    for child in node.children(&mut node.walk()) {
        match child.kind() {
            "assignment" => {
                if let Some(lhs) = child.child_by_field_name("left") {
                    if let Ok(name) = lhs.utf8_text(source.as_bytes()) {
                        let id = SymbolId::new(rel_path, class_module_path.to_vec(), name, 0);
                        let mut var_node = IrNode::new(
                            id.clone(),
                            NodeKind::Variable,
                            name,
                            node_location(node),
                        );
                        var_node.visibility = Visibility::Private;
                        var_node
                            .metadata
                            .insert("class_attribute".into(), serde_json::Value::Bool(true));
                        return Some(var_node);
                    }
                }
            }
            "annotated_assignment" => {
                if let Some(lhs) = child.child_by_field_name("left") {
                    if let Ok(name) = lhs.utf8_text(source.as_bytes()) {
                        let id = SymbolId::new(rel_path, class_module_path.to_vec(), name, 0);
                        let mut var_node = IrNode::new(
                            id.clone(),
                            NodeKind::Variable,
                            name,
                            node_location(node),
                        );
                        var_node.visibility = Visibility::Private;
                        var_node
                            .metadata
                            .insert("class_attribute".into(), serde_json::Value::Bool(true));
                        if let Some(type_ann) = child
                            .child_by_field_name("type")
                            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                        {
                            var_node.metadata.insert(
                                "type".into(),
                                serde_json::Value::String(type_ann.to_string()),
                            );
                        }
                        return Some(var_node);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

// ── Helper Utilities ───────────────────────────────────────────────────────

fn file_path_to_module_path(rel_path: &str) -> Vec<String> {
    let path = rel_path
        .strip_suffix(".py")
        .or_else(|| rel_path.strip_suffix(".pyi"))
        .or_else(|| rel_path.strip_suffix(".pyx"))
        .unwrap_or(rel_path);
    let normalized = path.replace('\\', "/");
    normalized
        .split('/')
        .filter(|s| *s != "__init__")
        .map(String::from)
        .collect()
}

fn node_location(node: &tree_sitter::Node) -> SourceLocation {
    let start = node.start_position();
    let end = node.end_position();
    SourceLocation {
        line_start: start.row as u32 + 1,
        line_end: end.row as u32 + 1,
        col_start: start.column as u32,
        col_end: end.column as u32,
    }
}

fn get_child_text<'a>(node: &tree_sitter::Node, source: &'a str, kind: &str) -> Option<&'a str> {
    node.child_by_field_name(kind)
        .or_else(|| find_child_of_kind(node, kind))
        .map(|n| n.utf8_text(source.as_bytes()).unwrap_or(""))
}

fn find_child_of_kind<'a>(
    node: &'a tree_sitter::Node,
    kind: &str,
) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

fn has_child_of_kind(node: &tree_sitter::Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return true;
        }
    }
    false
}

// ── Call Edge Extraction ───────────────────────────────────────────────────

fn extract_calls(
    node: &tree_sitter::Node,
    source: &str,
    rel_path: &str,
    caller_id: &SymbolId,
) -> Vec<IrEdge> {
    let mut edges = Vec::new();
    collect_call_edges(node, source, rel_path, caller_id, &mut edges);
    edges
}

fn collect_call_edges(
    node: &tree_sitter::Node,
    source: &str,
    rel_path: &str,
    caller_id: &SymbolId,
    edges: &mut Vec<IrEdge>,
) {
    if node.kind() == "call" {
        if let Some(func) = node.child_by_field_name("function") {
            let callee_name = match func.kind() {
                "identifier" => func.utf8_text(source.as_bytes()).ok().map(|s| s.to_string()),
                "attribute" => func.utf8_text(source.as_bytes()).ok().map(|s| s.to_string()),
                _ => None,
            };
            if let Some(ref name) = callee_name {
                let mut edge = IrEdge::new(
                    caller_id.clone(),
                    SymbolId::new(rel_path, Vec::new(), name, 0), // target_id is placeholder
                    EdgeKind::Calls,
                );
                // Store the actual callee name for name-based resolution
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
        collect_call_edges(&child, source, rel_path, caller_id, edges);
    }
}

// ── Import Extraction ──────────────────────────────────────────────────────

fn extract_imports(
    node: &tree_sitter::Node,
    source: &str,
    rel_path: &str,
    module_id: &SymbolId,
) -> Vec<IrEdge> {
    match node.kind() {
        "import_statement" => {
            // tree-sitter-python may wrap "from ... import ..." as both import_statement
            // and import_from_statement; skip the wrapping import_statement nodes.
            let node_text = node.utf8_text(source.as_bytes()).unwrap_or("").trim();
            if node_text.starts_with("from ") {
                return Vec::new();
            }

            let mut edges = Vec::new();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "dotted_name" || child.kind() == "aliased_import" {
                    if let Ok(name) = child.utf8_text(source.as_bytes()) {
                        let target_name = name
                            .split_whitespace()
                            .next()
                            .unwrap_or(name)
                            .trim()
                            .to_string();
                        if !target_name.is_empty() {
                            let target_id =
                                SymbolId::new(rel_path, Vec::new(), target_name, 0);
                            edges.push(IrEdge::new(
                                module_id.clone(),
                                target_id,
                                EdgeKind::Imports,
                            ));
                        }
                    }
                }
            }
            edges
        }
        "import_from_statement" => {
            let module_name = node
                .child_by_field_name("module_name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .unwrap_or("");
            let import_name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .unwrap_or("");
            let target_name = if module_name.is_empty() {
                import_name.to_string()
            } else if import_name.is_empty() {
                module_name.to_string()
            } else {
                import_name.to_string()
            };
            if target_name.is_empty() {
                return Vec::new();
            }
            let target_id = SymbolId::new(rel_path, Vec::new(), &target_name, 0);
            let mut edge = IrEdge::new(module_id.clone(), target_id, EdgeKind::Imports);
            if !module_name.is_empty() && !import_name.is_empty() {
                edge.metadata.insert(
                    "import_module".into(),
                    serde_json::Value::String(module_name.to_string()),
                );
            }
            vec![edge]
        }
        _ => Vec::new(),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn parse_source(source: &str) -> ParseResult {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let parser = PythonParser::new();
        let tmp = std::env::temp_dir();
        let file = tmp.join(format!("_ckc_test_{}.py", id));
        std::fs::write(&file, source).unwrap();
        let result = parser.parse_file(&tmp, &file).unwrap();
        // Clean up
        let _ = std::fs::remove_file(&file);
        result
    }

    #[test]
    fn parse_simple_function() {
        let result = parse_source("def hello():\n    print('world')\n");
        let functions: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(functions.len(), 1);
        assert_eq!(functions[0].name, "hello");
    }

    #[test]
    fn parse_class_with_method() {
        let result =
            parse_source("class Foo:\n    def bar(self):\n        baz()\n");
        let methods: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Method)
            .collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "bar");

        let has_call = result
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Calls && e.source_id.name == "bar");
        assert!(has_call);
    }

    #[test]
    fn parse_class_inheritance() {
        let result = parse_source("class Child(Parent):\n    pass\n");
        let has_inherits = result
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Inherits && e.target_id.name == "Parent");
        assert!(has_inherits);
    }

    #[test]
    fn parse_static_method() {
        let result = parse_source(
            "class Foo:\n    @staticmethod\n    def bar():\n        pass\n",
        );
        let methods: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Method)
            .collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "bar");
        assert_eq!(
            methods[0]
                .metadata
                .get("decorator")
                .and_then(|v| v.as_str()),
            Some("staticmethod")
        );
    }

    #[test]
    fn parse_property() {
        let result = parse_source(
            "class Foo:\n    @property\n    def x(self):\n        return 1\n",
        );
        let methods: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Method)
            .collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "x");
        assert_eq!(
            methods[0]
                .metadata
                .get("property")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn parse_docstring_function() {
        let result = parse_source(
            "def greet(name):\n    \"\"\"Say hello to someone.\"\"\"\n    print(f'Hello {name}')\n",
        );
        let funcs: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        let purpose = funcs[0]
            .semantic
            .as_ref()
            .and_then(|s| s.purpose.as_deref());
        assert_eq!(purpose, Some("Say hello to someone."));
    }

    #[test]
    fn parse_docstring_class() {
        let result = parse_source(
            "class Person:\n    \"\"\"Represents a person.\"\"\"\n    pass\n",
        );
        let classes: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Class)
            .collect();
        assert_eq!(classes.len(), 1);
        let purpose = classes[0]
            .semantic
            .as_ref()
            .and_then(|s| s.purpose.as_deref());
        assert_eq!(purpose, Some("Represents a person."));
    }

    #[test]
    fn parse_async_function() {
        let result = parse_source("async def fetch():\n    await something()\n");
        let funcs: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "fetch");
        assert_eq!(
            funcs[0]
                .metadata
                .get("async")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn parse_type_annotations() {
        let result = parse_source(
            "def add(a: int, b: int) -> int:\n    return a + b\n",
        );
        let funcs: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(
            funcs[0]
                .metadata
                .get("return_type")
                .and_then(|v| v.as_str()),
            Some("int")
        );
        // Signature hash should be non-zero when type annotations exist
        assert_ne!(funcs[0].id.signature_hash, 0);
    }

    #[test]
    fn parse_module_variable() {
        let result = parse_source("DEBUG = True\nVERSION: str = '1.0'\n");
        let vars: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Variable)
            .collect();
        assert_eq!(vars.len(), 2);
        assert!(vars.iter().any(|v| v.name == "DEBUG"));
        assert!(vars.iter().any(|v| v.name == "VERSION"));
    }

    #[test]
    fn parse_imports() {
        let result = parse_source("import os\nfrom collections import defaultdict\n");
        let imports: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert_eq!(imports.len(), 2);
        assert!(imports.iter().any(|e| e.target_id.name == "os"));
        // `from collections import defaultdict` stores the module in metadata
        assert!(imports.iter().any(|e| e.target_id.name == "defaultdict"
            && e.metadata.get("import_module").and_then(|v| v.as_str()) == Some("collections")));
    }

    #[test]
    fn parse_multi_import() {
        let result = parse_source("import os, sys, json\n");
        let imports: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert_eq!(imports.len(), 3);
        assert!(imports.iter().any(|e| e.target_id.name == "os"));
        assert!(imports.iter().any(|e| e.target_id.name == "sys"));
        assert!(imports.iter().any(|e| e.target_id.name == "json"));
    }

    #[test]
    fn parse_custom_decorator() {
        let result = parse_source(
            "@app.route('/')\ndef index():\n    return 'hello'\n",
        );
        let funcs: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        let decorators = funcs[0].metadata.get("decorators");
        assert!(decorators.is_some());
    }

    #[test]
    fn parse_stacked_decorators() {
        let result = parse_source(
            "@a\n@b\ndef foo():\n    pass\n",
        );
        let funcs: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        let decorators = funcs[0].metadata.get("decorators");
        assert!(decorators.is_some());
        let arr = decorators.unwrap().as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn parse_nested_function() {
        let result = parse_source(
            "def outer():\n    def inner():\n        pass\n    inner()\n",
        );
        let funcs: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 2);
        let names: Vec<&str> = funcs.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"outer"));
        assert!(names.contains(&"inner"));
        // inner should have a Contains edge from outer
        let contains = result
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Contains && e.target_id.name == "inner");
        assert!(contains);
    }

    #[test]
    fn parse_nested_class() {
        let result = parse_source(
            "class Outer:\n    class Inner:\n        pass\n",
        );
        let classes: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Class)
            .collect();
        assert_eq!(classes.len(), 2);
        let contains = result
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Contains
                && e.source_id.name == "Outer"
                && e.target_id.name == "Inner");
        assert!(contains);
    }

    #[test]
    fn parse_class_decorator() {
        let result = parse_source(
            "@dataclass\nclass Point:\n    x: int\n    y: int\n",
        );
        let classes: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Class)
            .collect();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Point");
        let decorators = classes[0].metadata.get("decorators");
        assert!(decorators.is_some());
    }
}
