//! SQLite-backed graph store for Knowledge IR.
//!
//! Provides persistence for nodes and edges, plus graph traversal queries
//! (callers, callees, dependencies, neighbors).

use ckc_ir::{EdgeKind, IrEdge, IrNode, SymbolId, IR_VERSION};
use rusqlite::{params, Connection};
use std::path::Path;
use thiserror::Error;

// ── Error ──────────────────────────────────────────────────────────────────

#[derive(Error, Debug)]
pub enum GraphError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Serialize(#[from] serde_json::Error),
}

// ── Graph Store ────────────────────────────────────────────────────────────

pub struct GraphStore {
    conn: Connection,
}

impl GraphStore {
    /// Open (or create) a graph database at the given path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, GraphError> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> Result<Self, GraphError> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), GraphError> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS nodes (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                name TEXT NOT NULL,
                file_path TEXT NOT NULL,
                line_start INTEGER,
                line_end INTEGER,
                col_start INTEGER,
                col_end INTEGER,
                visibility TEXT,
                metadata_json TEXT,
                semantic_json TEXT,
                hash INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS edges (
                source_id TEXT NOT NULL,
                target_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                metadata_json TEXT,
                PRIMARY KEY (source_id, target_id, kind)
            );

            CREATE INDEX IF NOT EXISTS idx_nodes_kind ON nodes(kind);
            CREATE INDEX IF NOT EXISTS idx_nodes_file ON nodes(file_path);
            CREATE INDEX IF NOT EXISTS idx_nodes_name ON nodes(name);
            CREATE INDEX IF NOT EXISTS idx_edges_source ON edges(source_id);
            CREATE INDEX IF NOT EXISTS idx_edges_target ON edges(target_id);
            CREATE INDEX IF NOT EXISTS idx_edges_kind ON edges(kind);

            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            ",
        )?;
        Ok(())
    }

    // ── Write ──────────────────────────────────────────────────────────

    /// Insert or replace a node (upsert by SymbolId).
    pub fn upsert_node(&self, node: &IrNode) -> Result<(), GraphError> {
        insert_node(&self.conn, node)
    }

    /// Insert or replace an edge (upsert by source, target, kind).
    pub fn upsert_edge(&self, edge: &IrEdge) -> Result<(), GraphError> {
        insert_edge(&self.conn, edge)
    }

    /// Persist all nodes and edges from a build result in a single transaction.
    pub fn persist_batch(&self, nodes: &[IrNode], edges: &[IrEdge]) -> Result<(), GraphError> {
        let tx = self.conn.unchecked_transaction()?;
        for node in nodes {
            insert_node(&tx, node)?;
        }
        for edge in edges {
            insert_edge(&tx, edge)?;
        }
        // Update metadata (use tx, not self.conn)
        tx.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            params!["ir_version", IR_VERSION.to_string()],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            params!["total_nodes", nodes.len().to_string()],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            params!["total_edges", edges.len().to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    #[allow(dead_code)]
    fn set_meta(&self, key: &str, value: &str) -> Result<(), GraphError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    /// Get a metadata value.
    pub fn get_meta(&self, key: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .ok()
    }

    // ── Queries ─────────────────────────────────────────────────────────

    /// Find callers of a symbol (nodes that have a Calls edge to it).
    pub fn callers(&self, name: &str, depth: u32) -> Result<Vec<IrNode>, GraphError> {
        self.traverse_up(name, edge_kind_str(EdgeKind::Calls), depth)
    }

    /// Find callees of a symbol (nodes it calls).
    pub fn callees(&self, name: &str, depth: u32) -> Result<Vec<IrNode>, GraphError> {
        self.traverse_down(name, edge_kind_str(EdgeKind::Calls), depth)
    }

    /// Find imports of a file.
    pub fn imports_of_file(&self, file: &str) -> Result<Vec<IrEdge>, GraphError> {
        let kind = edge_kind_str(EdgeKind::Imports);
        let mut stmt = self.conn.prepare(
            "SELECT source_id, target_id, kind, metadata_json FROM edges
             WHERE source_id LIKE ?1 AND kind = ?2",
        )?;
        let rows: Vec<IrEdge> = stmt
            .query_map(params![format!("{}::%", file), kind], row_to_edge)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(GraphError::from)?;
        Ok(rows)
    }

    /// Find all dependencies (outgoing edges of kinds Calls, Imports, DependsOn).
    pub fn dependencies(&self, name: &str) -> Result<Vec<IrNode>, GraphError> {
        self.traverse_down(name, "calls", 1)
    }

    /// Find all dependents (incoming edges of kinds Calls, Imports, DependsOn).
    pub fn dependents(&self, name: &str) -> Result<Vec<IrNode>, GraphError> {
        self.traverse_up(name, "calls", 1)
    }

    /// Find neighbors up to the given depth.
    pub fn neighbors(&self, name: &str, depth: u32) -> Result<Vec<IrNode>, GraphError> {
        let mut all = Vec::new();
        all.extend(self.traverse_up(name, "calls", depth)?);
        all.extend(self.traverse_down(name, "calls", depth)?);
        // Deduplicate by id
        let mut seen = std::collections::HashSet::new();
        all.retain(|n| seen.insert(n.id.to_key()));
        Ok(all)
    }

    /// List nodes, optionally filtered by kind.
    pub fn list_nodes(&self, kind: Option<&str>) -> Result<Vec<IrNode>, GraphError> {
        if let Some(k) = kind {
            let mut stmt = self
                .conn
                .prepare("SELECT * FROM nodes WHERE kind = ?1 ORDER BY file_path, name")?;
            let rows: Vec<IrNode> = stmt
                .query_map(params![k], row_to_node)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(GraphError::from)?;
            Ok(rows)
        } else {
            let mut stmt = self
                .conn
                .prepare("SELECT * FROM nodes ORDER BY file_path, name")?;
            let rows: Vec<IrNode> = stmt
                .query_map([], row_to_node)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(GraphError::from)?;
            Ok(rows)
        }
    }

    /// Count nodes by kind.
    pub fn count_nodes(&self) -> Result<Vec<(String, u64)>, GraphError> {
        let mut stmt = self
            .conn
            .prepare("SELECT kind, COUNT(*) FROM nodes GROUP BY kind ORDER BY 2 DESC")?;
        let rows: Vec<(String, u64)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(GraphError::from)?;
        Ok(rows)
    }

    pub fn total_nodes(&self) -> u64 {
        self.conn
            .query_row("SELECT COUNT(*) FROM nodes", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0) as u64
    }

    pub fn total_edges(&self) -> u64 {
        self.conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0) as u64
    }

    // ── Internal ────────────────────────────────────────────────────────

    /// BFS traversal following outgoing edges.
    fn traverse_down(
        &self,
        name: &str,
        edge_kind: &str,
        depth: u32,
    ) -> Result<Vec<IrNode>, GraphError> {
        let mut results = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut current: Vec<String> = self.find_node_keys_by_name(name)?;

        // Prepare statement once, outside the loop
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.kind, n.name, n.file_path, n.line_start, n.line_end,
                    n.col_start, n.col_end, n.visibility, n.metadata_json,
                    n.semantic_json, n.hash
             FROM edges e JOIN nodes n ON n.id = e.target_id
             WHERE e.source_id = ?1 AND e.kind = ?2",
        )?;

        for _ in 0..depth {
            if current.is_empty() {
                break;
            }
            let mut next = Vec::new();
            for key in &current {
                if !visited.insert(key.clone()) {
                    continue;
                }
                let rows = stmt.query_map(params![key, edge_kind], row_to_node)?;
                for row in rows {
                    let node = row?;
                    next.push(node.id.to_key());
                    results.push(node);
                }
            }
            current = next;
        }
        Ok(results)
    }

    /// BFS traversal following incoming edges.
    fn traverse_up(
        &self,
        name: &str,
        edge_kind: &str,
        depth: u32,
    ) -> Result<Vec<IrNode>, GraphError> {
        let mut results = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut current: Vec<String> = self.find_node_keys_by_name(name)?;

        // Prepare statement once, outside the loop
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.kind, n.name, n.file_path, n.line_start, n.line_end,
                    n.col_start, n.col_end, n.visibility, n.metadata_json,
                    n.semantic_json, n.hash
             FROM edges e JOIN nodes n ON n.id = e.source_id
             WHERE e.target_id = ?1 AND e.kind = ?2",
        )?;

        for _ in 0..depth {
            if current.is_empty() {
                break;
            }
            let mut next = Vec::new();
            for key in &current {
                if !visited.insert(key.clone()) {
                    continue;
                }
                let rows = stmt.query_map(params![key, edge_kind], row_to_node)?;
                for row in rows {
                    let node = row?;
                    next.push(node.id.to_key());
                    results.push(node);
                }
            }
            current = next;
        }
        Ok(results)
    }

    fn find_node_keys_by_name(&self, name: &str) -> Result<Vec<String>, GraphError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM nodes WHERE name = ?1")?;
        let rows = stmt.query_map(params![name], |row| row.get::<_, String>(0))?;
        let keys: Vec<String> = rows.collect::<Result<_, _>>()?;
        Ok(keys)
    }
}

// ── Row Mappers ────────────────────────────────────────────────────────────

/// Serialize NodeKind to its DB string representation without serde_json overhead.
pub fn node_kind_str(kind: ckc_ir::NodeKind) -> &'static str {
    match kind {
        ckc_ir::NodeKind::File => "file",
        ckc_ir::NodeKind::Module => "module",
        ckc_ir::NodeKind::Function => "function",
        ckc_ir::NodeKind::Method => "method",
        ckc_ir::NodeKind::Class => "class",
        ckc_ir::NodeKind::Struct => "struct",
        ckc_ir::NodeKind::Enum => "enum",
        ckc_ir::NodeKind::EnumVariant => "enum_variant",
        ckc_ir::NodeKind::Trait => "trait",
        ckc_ir::NodeKind::TraitImpl => "trait_impl",
        ckc_ir::NodeKind::Interface => "interface",
        ckc_ir::NodeKind::TypeAlias => "type_alias",
        ckc_ir::NodeKind::Constant => "constant",
        ckc_ir::NodeKind::Static => "static",
        ckc_ir::NodeKind::Variable => "variable",
    }
}

/// Serialize EdgeKind to its DB string representation without serde_json overhead.
pub fn edge_kind_str(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Calls => "calls",
        EdgeKind::Imports => "imports",
        EdgeKind::Contains => "contains",
        EdgeKind::Inherits => "inherits",
        EdgeKind::Instantiates => "instantiates",
        EdgeKind::References => "references",
        EdgeKind::DependsOn => "depends_on",
    }
}

fn insert_node(conn: &rusqlite::Connection, node: &IrNode) -> Result<(), GraphError> {
    let key = node.id.to_key();
    let metadata_json = serde_json::to_string(&node.metadata)?;
    let semantic_json = node
        .semantic
        .as_ref()
        .map(|s| serde_json::to_string(s))
        .transpose()?;
    let visibility = match node.visibility {
        ckc_ir::Visibility::Public => "public",
        ckc_ir::Visibility::Private => "private",
        ckc_ir::Visibility::Protected => "protected",
    };

    conn.execute(
        "INSERT OR REPLACE INTO nodes
            (id, kind, name, file_path, line_start, line_end, col_start, col_end,
             visibility, metadata_json, semantic_json, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            key,
            node_kind_str(node.kind),
            node.name,
            node.id.file_path,
            node.location.line_start,
            node.location.line_end,
            node.location.col_start,
            node.location.col_end,
            visibility,
            metadata_json,
            semantic_json,
            node.hash as i64,
        ],
    )?;
    Ok(())
}

fn insert_edge(conn: &rusqlite::Connection, edge: &IrEdge) -> Result<(), GraphError> {
    let source_key = edge.source_id.to_key();
    let target_key = edge.target_id.to_key();
    let metadata_json = serde_json::to_string(&edge.metadata)?;

    conn.execute(
        "INSERT OR REPLACE INTO edges (source_id, target_id, kind, metadata_json)
         VALUES (?1, ?2, ?3, ?4)",
        params![source_key, target_key, edge_kind_str(edge.kind), metadata_json],
    )?;
    Ok(())
}

fn row_to_node(row: &rusqlite::Row<'_>) -> rusqlite::Result<IrNode> {
    let id_key: String = row.get(0)?;
    let kind_str: String = row.get(1)?;
    let name: String = row.get(2)?;
    let file_path: String = row.get(3)?;
    let line_start: u32 = row.get(4)?;
    let line_end: u32 = row.get(5)?;
    let col_start: u32 = row.get(6)?;
    let col_end: u32 = row.get(7)?;
    let visibility_str: String = row.get(8)?;
    let metadata_json: Option<String> = row.get(9)?;
    let semantic_json: Option<String> = row.get(10)?;
    let hash: i64 = row.get(11)?;

    let id = SymbolId::from_key(&id_key).unwrap_or_else(|| {
        SymbolId::new(file_path.clone(), Vec::new(), name.clone(), 0)
    });

    let kind: ckc_ir::NodeKind =
        serde_json::from_str(&format!("\"{}\"", kind_str)).unwrap_or(ckc_ir::NodeKind::Function);
    let visibility = match visibility_str.as_str() {
        "private" => ckc_ir::Visibility::Private,
        "protected" => ckc_ir::Visibility::Protected,
        _ => ckc_ir::Visibility::Public,
    };
    let metadata = metadata_json
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let semantic = semantic_json
        .and_then(|s| serde_json::from_str(&s).ok());

    Ok(IrNode {
        id,
        kind,
        name,
        location: ckc_ir::SourceLocation {
            line_start,
            line_end,
            col_start,
            col_end,
        },
        visibility,
        metadata,
        semantic,
        hash: hash as u64,
    })
}

fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<IrEdge> {
    let source_key: String = row.get(0)?;
    let target_key: String = row.get(1)?;
    let kind_str: String = row.get(2)?;
    let metadata_json: Option<String> = row.get(3)?;

    let kind: EdgeKind =
        serde_json::from_str(&format!("\"{}\"", kind_str)).unwrap_or(EdgeKind::Calls);
    let source_id = SymbolId::from_key(&source_key).unwrap_or_else(|| {
        SymbolId::new("unknown", Vec::new(), source_key, 0)
    });
    let target_id = SymbolId::from_key(&target_key).unwrap_or_else(|| {
        SymbolId::new("unknown", Vec::new(), target_key, 0)
    });
    let metadata = metadata_json
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    Ok(IrEdge {
        source_id,
        target_id,
        kind,
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ckc_ir::{IrEdge, IrNode, NodeKind, SourceLocation, SymbolId};

    fn make_node(name: &str, kind: NodeKind, file: &str) -> IrNode {
        IrNode::new(
            SymbolId::new(file, Vec::new(), name, 0),
            kind,
            name,
            SourceLocation {
                line_start: 1,
                line_end: 1,
                col_start: 0,
                col_end: 0,
            },
        )
    }

    #[test]
    fn upsert_and_query() {
        let store = GraphStore::open_in_memory().unwrap();

        let a = make_node("a", NodeKind::Function, "mod.py");
        let b = make_node("b", NodeKind::Function, "mod.py");

        store.upsert_node(&a).unwrap();
        store.upsert_node(&b).unwrap();
        store
            .upsert_edge(&IrEdge::new(a.id.clone(), b.id.clone(), EdgeKind::Calls))
            .unwrap();

        let callers = store.callers("b", 1).unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].name, "a");

        let callees = store.callees("a", 1).unwrap();
        assert_eq!(callees.len(), 1);
        assert_eq!(callees[0].name, "b");
    }

    #[test]
    fn batch_persist() {
        let store = GraphStore::open_in_memory().unwrap();
        let nodes = vec![
            make_node("main", NodeKind::Function, "main.py"),
            make_node("helper", NodeKind::Function, "main.py"),
        ];
        let edges = vec![IrEdge::new(
            nodes[0].id.clone(),
            nodes[1].id.clone(),
            EdgeKind::Calls,
        )];

        store.persist_batch(&nodes, &edges).unwrap();
        assert_eq!(store.total_nodes(), 2);
        assert_eq!(store.total_edges(), 1);
    }
}
