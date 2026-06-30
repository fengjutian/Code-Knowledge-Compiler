# CKC — Code Knowledge Compiler

**Compile Code into Queryable Knowledge.**

CKC compiles a Python codebase into a Knowledge IR — a graph of nodes
(functions, classes, modules, variables) and edges (calls, imports,
inheritance, containment) — stored in SQLite and queryable via CLI.

> 🚧 **Phase 1 MVP** — Python-only, structural graph queries.

## Quick Start

```bash
# Build CKC
cargo build --release

# Scan a Python project
ckc scan ./my-project

# Compile to Knowledge IR
ckc build ./my-project

# Query the knowledge graph
ckc query callers   -n process
ckc query callees   -n main
ckc query imports   -f src/app.py
ckc query neighbors -n UserModel --depth 2
ckc query list-nodes -k class

# View build statistics
ckc status ./my-project
```

## Architecture

```
Repository → Scanner → Parser (tree-sitter) → Knowledge IR → SQLite → CLI Queries
```

### Workspace Crates

| Crate | Purpose |
|-------|---------|
| `ckc-ir` | Core types: SymbolId, IrNode, IrEdge, SemanticInfo |
| `ckc-parser` | Python parser via tree-sitter-python |
| `ckc-graph` | SQLite-backed graph store with BFS traversal |
| `ckc-core` | Scanner (.gitignore-aware) + compilation orchestrator |
| `ckc-cli` | Command-line interface (clap) |

## Phase 1 Features

- Python source parsing (functions, classes, methods, variables)
- Edge extraction (calls, imports, inheritance, containment)
- Decorator recognition (@staticmethod, @classmethod, @property, custom)
- Docstring → purpose extraction
- Type annotations → metadata + signature hashing
- Async function detection
- Nested function/class support
- SQLite persistence with WAL mode
- Graph traversal queries (BFS, callers, callees, neighbors)
- .gitignore-aware file scanning

## License

MIT OR Apache-2.0
