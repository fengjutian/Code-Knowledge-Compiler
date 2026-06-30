# CKC (Code Knowledge Compiler) — Product Requirements Document

> **Status**: Draft v0.1
> **Target**: Phase 1 MVP
> **Complement to**: `requirement.md` (vision & architecture)

---

## 1. Functional Requirements

### 1.1 Repository Scanner (`ckc-scanner`)

| ID | Requirement | Priority |
|----|-------------|----------|
| **FR-SCN-001** | 递归扫描指定目录，识别所有源代码文件 | P0 |
| **FR-SCN-002** | 支持 `.gitignore` 风格的文件排除规则 | P0 |
| **FR-SCN-003** | 根据文件扩展名自动检测语言类型 | P0 |
| **FR-SCN-004** | 维护文件级的 content hash（SHA-256），用于增量变更检测 | P0 |
| **FR-SCN-005** | 支持手动指定语言覆盖（如 `.h` 是 C 还是 C++） | P2 |

### 1.2 Language Parser (`ckc-parser`)

| ID | Requirement | Priority |
|----|-------------|----------|
| **FR-PRS-001** | 基于 tree-sitter 解析 Python（Phase 1 唯一支持的语言） | P0 |
| **FR-PRS-002** | 从 CST 提取符号表：Function、Class、Struct、Trait、Interface、Enum、Module、Variable | P0 |
| **FR-PRS-003** | 提取以下边关系：Calls、Imports、Contains、Inherits（Python 类继承） | P0 |
| **FR-PRS-004** | 为每个符号生成稳定的 SymbolId（路径 + 名称 + 签名 hash） | P0 |
| **FR-PRS-005** | 解析失败时产出部分结果（best-effort），标记解析错误 | P1 |
| **FR-PRS-006** | 使用 rowan 红绿树作为统一 CST 内存表示（为多语言扩展做准备） | P2 |

### 1.3 Knowledge IR (`ckc-ir`)

| ID | Requirement | Priority |
|----|-------------|----------|
| **FR-IR-001** | 定义 Node 类型系统，每个 Node 有唯一 ID、类型标签、名称、位置、所属文件 | P0 |
| **FR-IR-002** | 定义 Edge 类型系统，每条边有 source node、target node、edge kind、元数据 | P0 |
| **FR-IR-003** | 支持 Node 和 Edge 上的附加属性（Metadata map） | P0 |
| **FR-IR-004** | 支持序列化/反序列化（Phase 1：MessagePack 或 bincode） | P0 |
| **FR-IR-005** | IR 版本号，支持向前兼容检查 | P1 |

### 1.4 Graph Store & Query (`ckc-graph` / `ckc-query`)

| ID | Requirement | Priority |
|----|-------------|----------|
| **FR-GPH-001** | 将 IR 持久化到 SQLite（nodes 表 + edges 表） | P0 |
| **FR-GPH-002** | 支持按 node 类型过滤查询 | P0 |
| **FR-GPH-003** | 支持图遍历查询：callers、callees、dependencies、dependents | P0 |
| **FR-GPH-004** | 支持 BFS/DFS 指定深度的邻域查询 | P0 |
| **FR-GPH-005** | 支持通过 SQLite FTS5 对符号名和文件路径进行全文搜索 | P1 |
| **FR-GPH-006** | 支持 path-finding 查询（A → B 的最短调用路径） | P2 |

### 1.5 Semantic Compiler (`ckc-semantic`)

| ID | Requirement | Priority |
|----|-------------|----------|
| **FR-SEM-001** | 计算每个函数的圈复杂度（Cyclomatic Complexity） | P1 |
| **FR-SEM-002** | 计算每个模块的耦合度（fan-in / fan-out） | P1 |
| **FR-SEM-003** | 从代码注释中提取 summary（以 `///` 或 `/** */` 开头）作为 Purpose 字段 | P1 |
| **FR-SEM-004** | 通过 LLM 为函数/类生成 Purpose、Responsibility 摘要 | P2 |
| **FR-SEM-005** | 通过 LLM 识别 Business Capability 标签 | P2 |

### 1.6 Vector Store (`ckc-vector`) — Phase 2

| ID | Requirement | Priority |
|----|-------------|----------|
| **FR-VEC-001** | 使用 Qdrant 存储函数/类的 embedding 向量 | P2 |
| **FR-VEC-002** | 支持基于 embedding 的语义相似搜索 | P2 |
| **FR-VEC-003** | 支持 hybrid search（向量相似 + 图结构约束） | P2 |

### 1.7 CLI (`ckc-cli`)

| ID | Requirement | Priority |
|----|-------------|----------|
| **FR-CLI-001** | `ckc scan <path>` — 扫描仓库并输出文件清单 | P0 |
| **FR-CLI-002** | `ckc build <path>` — 执行全量编译，产出 IR 并持久化 | P0 |
| **FR-CLI-003** | `ckc query <query>` — 对已构建的 IR 执行图查询 | P0 |
| **FR-CLI-004** | `ckc status` — 显示 IR 统计信息（文件数、节点数、边数、构建时间） | P0 |
| **FR-CLI-005** | `ckc serve` — 启动本地 HTTP API | P2 |

### 1.8 Incremental Compilation (`ckc-core`)

| ID | Requirement | Priority |
|----|-------------|----------|
| **FR-INC-001** | 基于 file hash 检测变更文件 | P1 |
| **FR-INC-002** | 仅重新编译变更文件及其直接影响域（salsa 风格 query invalidation） | P1 |
| **FR-INC-003** | 增量更新 SQLite 中的 node/edge（upsert by SymbolId） | P1 |

---

## 2. Non-Functional Requirements

### 2.1 Performance

| ID | Metric | Target | Priority |
|----|--------|--------|----------|
| **NFR-PERF-001** | 全量编译吞吐 | ≥ 10K LOC/s（单语言 Rust） | P0 |
| **NFR-PERF-002** | 增量编译延迟 | ≤ 500ms（单文件变更后 IR 更新） | P1 |
| **NFR-PERF-003** | 图查询延迟 | ≤ 200ms（简单遍历）、≤ 2s（BFS 深度 5） | P1 |
| **NFR-PERF-004** | 向量检索延迟 | ≤ 100ms（top-10 检索） | P1 |
| **NFR-PERF-005** | 内存使用 | ≤ 4GB（10 万文件仓库全量编译峰值） | P2 |
| **NFR-PERF-006** | 磁盘占用（IR） | ≤ 源码大小的 2x（不含 embedding） | P2 |

### 2.2 Reliability

| ID | Requirement | Priority |
|----|-------------|----------|
| **NFR-REL-001** | 单文件解析失败不影响其余文件的编译（isolation） | P0 |
| **NFR-REL-002** | `ckc build` 过程可中断恢复（幂等写入 SQLite） | P1 |
| **NFR-REL-003** | IR 损坏时提供 `ckc rebuild --force` 重建 | P1 |

### 2.3 Compatibility

| ID | Requirement | Priority |
|----|-------------|----------|
| **NFR-COMP-001** | 支持 Windows、macOS、Linux | P0 |
| **NFR-COMP-002** | Rust toolchain：stable（MSRV 跟随最新稳定版） | P0 |
| **NFR-COMP-003** | SQLite ≥ 3.35（FT5 需要 3.35+；递归 CTE 需要 3.8+） | P0 |
| **NFR-COMP-004** | Qdrant ≥ 1.0（Phase 2，gRPC 或 REST） | P2 |

---

## 3. Knowledge IR Schema Specification

### 3.1 Node Types

```rust
/// 全局唯一符号标识
pub struct SymbolId {
    pub file_path: String,       // 相对于仓库根
    pub module_path: Vec<String>, // 模块路径，如 ["crate", "module"]
    pub name: String,            // 符号名
    pub signature_hash: u64,     // 签名的 XXH3 hash（用于区分重载）
}

/// Node 类型枚举
pub enum NodeKind {
    File,
    Module,
    Function,
    Method,
    Struct,
    Enum,
    EnumVariant,
    Trait,
    TraitImpl,
    Interface,       // TypeScript
    Class,           // Python/TypeScript
    TypeAlias,
    Constant,
    Static,
    Variable,        // 全局变量 / 模块级变量
}

/// Knowledge IR 中的节点
pub struct IrNode {
    pub id: SymbolId,
    pub kind: NodeKind,
    pub name: String,
    pub location: SourceLocation,
    pub visibility: Visibility,
    pub metadata: HashMap<String, serde_json::Value>,
    pub semantic: Option<SemanticInfo>,
    pub hash: u64,             // 此节点的 content hash（用于增量）
}
```

### 3.2 Edge Types

```rust
pub enum EdgeKind {
    Calls,           // 函数/方法调用
    Imports,         // 模块导入
    Contains,        // 模块包含函数/类/子模块
    Inherits,        // 类继承 / trait 实现
    Instantiates,    // 创建实例
    References,      // 通用引用（变量引用、类型引用）
    DependsOn,       // 模块/文件级依赖
}
```

### 3.3 Semantic Info

```rust
pub struct SemanticInfo {
    pub purpose: Option<String>,             // 函数的业务目的（单句摘要）
    pub summary: Option<String>,             // 详细摘要（3-5 句）
    pub responsibility: Vec<String>,         // 职责标签
    pub business_capability: Vec<String>,    // 业务能力标签，如 "payment", "auth"
    pub design_pattern: Vec<String>,         // 设计模式，如 "Singleton", "Factory"
    pub complexity: Option<ComplexityMetrics>,
    pub risks: Vec<RiskTag>,                 // 风险标签
}

pub struct ComplexityMetrics {
    pub cyclomatic: u32,           // 圈复杂度
    pub lines_of_code: u32,
    pub fan_in: u32,               // 被多少个外部函数调用
    pub fan_out: u32,              // 调用了多少个外部函数
}

pub struct RiskTag {
    pub severity: RiskSeverity,    // Low / Medium / High / Critical
    pub category: String,          // "NoAuth", "UnsafeCode", "SQLInjection", ...
    pub description: String,
}
```

### 3.4 Embedding Info

```rust
pub struct EmbeddingInfo {
    pub model_name: String,         // 生成 embedding 的模型
    pub model_version: String,
    pub vector: Vec<f32>,
    pub dimensions: u32,
    pub input_text_hash: u64,      // 生成向量时输入文本的 hash（用于增量检测）
}
```

### 3.5 IR 持久化布局（SQLite）

```sql
CREATE TABLE nodes (
    id TEXT PRIMARY KEY,            -- SymbolId 的序列化字符串
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

CREATE TABLE edges (
    source_id TEXT NOT NULL,
    target_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    metadata_json TEXT,
    PRIMARY KEY (source_id, target_id, kind)
);

CREATE INDEX idx_nodes_kind ON nodes(kind);
CREATE INDEX idx_nodes_file ON nodes(file_path);
CREATE INDEX idx_nodes_name ON nodes(name);
CREATE INDEX idx_edges_source ON edges(source_id);
CREATE INDEX idx_edges_target ON edges(target_id);
CREATE INDEX idx_edges_kind ON edges(kind);

CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- Meta keys: ir_version, build_timestamp, repo_root, total_files, total_nodes, total_edges
```

---

## 4. Parser Pipeline Specification

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│ tree-sitter  │────▶│  ckc-parser  │────▶│  IR Builder   │
│ (Python CST) │     │ (符号提取)    │     │ (节点+边)      │
└──────────────┘     └──────────────┘     └──────┬───────┘
                                                  │
                                                  ▼
                                           ┌──────────────┐
                                           │  Knowledge IR │
                                           │  (final)       │
                                           └──────────────┘
```

> Note: Phase 1 暂不在 tree-sitter 和 ckc-parser 之间插入 rowan 层。rowan 的多语言统一 CST 表示的收益在单语言阶段不显著，推迟到 Phase 2 多语言时引入。

### 4.1 tree-sitter 语言绑定

- **Python (Phase 1)**: `tree-sitter-python` — 官方维护，质量成熟
- **Rust (Phase 2)**: `tree-sitter-rust` + `syn`（syn 提供完整 Rust AST 语义，用于 Trait impl 解析等需要类型信息的部分）
- **TypeScript (Phase 2)**: `tree-sitter-typescript`（含 TSX）

### 4.2 符号提取策略（Python Phase 1）

| 提取目标 | 方法 |
|----------|------|
| **Function** | tree-sitter query `(function_definition name: (identifier))` |
| **Class** | tree-sitter query `(class_definition name: (identifier))` |
| **Method** | tree-sitter query `(class_definition body: (block (function_definition name: (identifier))))` |
| **Calls** | tree-sitter query `(call function: (identifier))` 和 `(call function: (attribute))` |
| **Imports** | tree-sitter query `(import_statement)` / `(import_from_statement)` |
| **Inherits** | tree-sitter query `(class_definition superclasses: (argument_list))` |
| **Module** | 文件即 Module（Python 惯例：一个 `.py` 文件 = 一个 module） |

---

## 5. Query Runtime Specification

### 5.1 Query CLI (Phase 1 — Structured)

Phase 1 使用结构化 CLI 子命令，而非自然语言。自然语言 intent parser 推迟到 Phase 2（需 LLM）。

```
ckc query callers    -n <name> [--depth <n>]     # "谁调用了 X？"
ckc query callees    -n <name> [--depth <n>]     # "X 调用了谁？"
ckc query imports    -f <file>                   # "X 文件导入了什么？"
ckc query imported-by -f <file>                  # "谁导入了 X 文件？"
ckc query dependencies -n <name> [--direction both|in|out]  # "X 的依赖和被依赖"
ckc query path       --from <a> --to <b>          # "A 到 B 的调用路径？"
ckc query neighbors  -n <name> [--depth <n>]      # "X 的邻域子图"
ckc query list nodes   [--kind <kind>]            # 列出指定类型的节点
ckc query list edges   [--kind <kind>]            # 列出指定类型的边
```

### 5.2 Query Types (Phase 2+ — with LLM + Vector)

```
QUERY
  ├── structural    (图结构查询)
  │     ├── callers      "谁调用了 X？"
  │     ├── callees      "X 调用了谁？"
  │     ├── imports      "X 导入了什么？"
  │     ├── containment  "X 包含哪些符号？"
  │     └── path         "A 到 B 的调用路径？"
  │
  ├── semantic     (语义查询)
  │     ├── by_purpose   "哪些函数处理支付？"
  │     ├── by_tag       "标记为 auth 的模块？"
  │     └── by_risk      "有哪些高风险节点？"
  │
  └── hybrid       (混合查询)
        ├── graph + vector    "与 X 相似且被 X 调用的函数？"
        └── graph + semantic  "支付模块中圈复杂度 > 10 的函数？"
```

### 5.3 查询输出格式

```json
{
  "query": "callers of PaymentService",
  "nodes": [...],
  "edges": [...],
  "context": {
    "call_depth": 2,
    "total_related_nodes": 15
  }
}
```

输出可直接作为 LLM context（配合 `ckc-llm` 的 Context Builder）。

---

## 6. Workspace Crate Responsibilities

| Crate | 职责 | 对外 Trait | 依赖 |
|-------|------|-----------|------|
| **ckc-core** | Scanner + 编译调度 + 增量编排 + 公共类型 | `Scanner`, `Compiler`, `IncrementalChecker` | ckc-ir |
| **ckc-parser** | Python 解析 + tree-sitter-python 集成（多语言抽象预留 Trait） | `LanguageParser`, `SymbolExtractor` | ckc-ir, tree-sitter |
| **ckc-ir** | Knowledge IR 类型定义 + 序列化 | (pure types, no trait) | serde, bincode |
| **ckc-graph** | Graph store（SQLite）读写 + 图遍历算法 | `GraphStore`, `GraphQuery` | ckc-ir, rusqlite |
| **ckc-vector** | Embedding 生成 + Qdrant 集成（Phase 2） | `EmbeddingGenerator`, `VectorStore` | ckc-ir |
| **ckc-semantic** | 语义分析（复杂度、耦合、注释提取） | `SemanticAnalyzer` | ckc-ir |
| **ckc-query** | Query CLI 解析 + Graph 查询执行 + 结果格式化 | `QueryEngine`, `ContextBuilder` | ckc-graph, ckc-semantic |
| **ckc-llm** | LLM 调用抽象（Purpose 生成、Summary） | `LlmCompiler` | ckc-ir |
| **ckc-storage** | 持久化抽象（Save/Load/Update 接口） | `StorageBackend` | ckc-ir |
| **ckc-runtime** | 协调层：Server/Agent 模式下的查询调度 | (暂不定义，Phase 2+) | ckc-query |
| **ckc-cli** | 命令行入口 | (binary) | ckc-core, ckc-graph, ckc-query |

---

## 7. Phase 1 MVP — Acceptance Criteria

### 7.1 Must Have (P0)

- [ ] `ckc scan <path>` 扫描 Python 仓库并打印文件清单
- [ ] `ckc build <path>` 编译 Python 仓库，产出 Knowledge IR 并持久化到 SQLite
- [ ] IR 覆盖以下 Node 类型：File, Module, Function, Method, Class, Variable
- [ ] IR 覆盖以下 Edge 类型：Calls, Imports, Contains, Inherits
- [ ] `ckc query callers -n <name>` 返回调用者列表
- [ ] `ckc query callees -n <name>` 返回被调用者列表
- [ ] `ckc query imports -f <file>` 返回模块导入关系
- [ ] `ckc query dependencies -n <name>` 返回依赖和被依赖关系
- [ ] `ckc status` 显示 IR 统计信息
- [ ] 基于 `tree-sitter-python` 的完整 Python 解析集成

### 7.2 Nice to Have (P1)

- [ ] 多文件模块（`__init__.py` 目录）支持
- [ ] 递归 import 解析（import chain 展开）
- [ ] SQLite FTS5 符号名搜索
- [ ] 圈复杂度计算
- [ ] `ckc build` 幂等性（重复运行不重复插入）
- [ ] 部分失败隔离（单文件报错不影响整体）

### 7.3 Out of Scope (Phase 2+)

- [ ] 增量编译
- [ ] Rust / TypeScript 支持
- [ ] LLM 语义摘要生成
- [ ] Qdrant 向量存储 + 语义搜索
- [ ] HTTP API / MCP Server
- [ ] IDE 插件
- [ ] 自然语言查询（Phase 1 使用结构化 CLI 子命令）

---

## 8. Technical Decisions & Rationale

| 决策 | 选择 | 理由 |
|------|------|------|
| **Phase 1 仅 Python** | `tree-sitter-python` | Python 是 tree-sitter 官方 Tier-1 语法，质量成熟。Python 的 import 模型简单（文件=模块），符号提取无需复杂的类型推断。单语言先验证 IR 和查询链路的正确性 |
| **IR 序列化** | bincode（内部）+ JSON（debug/export） | bincode 零开销反序列化，适合 Rust-Rust 通信；JSON 适合 CLI 输出和外部系统消费 |
| **Graph DB** | SQLite（而非 Neo4j） | 零外部依赖，嵌入式部署，递归 CTE 满足 Phase 1 图遍历需求。企业级可后续接入专用图数据库 |
| **构建模型** | 全量编译 → Phase 1；增量编译 → Phase 2 | 先验证 IR 正确性和查询能力，再增加 incremental 复杂度 |
| **符号 ID** | 路径 + 名称 + 签名 hash | 稳定、可读、冲突概率极低。比自增 ID 更适合增量更新 |
| **Query 语法 (Phase 1)** | 结构化 CLI 子命令（如 `ckc query callers -n foo`） | 自然语言 intent parser 需要 LLM，推迟到 Phase 2。结构化命令确定性高、可脚本化 |
| **CLI 框架** | clap derive | Rust 生态标准 |
| **日志** | tracing | 结构化日志，支持 span（适合编译器多阶段追踪） |
| **错误处理** | anyhow（应用层）+ thiserror（库层） | Rust 惯例 |
| **Embedding 模型 (Phase 2)** | 默认本地 fastembed，可选远端 API | 开箱即用（零外部依赖），用户可覆盖为 OpenAI/Cohere |

---

## 9. Glossary

| 术语 | 定义 |
|------|------|
| **Knowledge IR** | CKC 的核心中间表示，包含 Node、Edge、Semantic、Embedding、Metadata 五类信息 |
| **Syntax Compiler (①)** | 四级编译器的第一级：源码 → AST/CST → 符号 + 关系 |
| **Semantic Compiler (②)** | 四级编译器的第二级：AST → 业务语义（Purpose、Responsibility、Design Pattern） |
| **Knowledge Compiler (③)** | 四级编译器的第三级：语义信息 + Embedding → 可查询知识图谱 |
| **Query Compiler (④)** | 四级编译器的第四级：自然语言/结构化查询 → 图遍历 + 向量检索 + 上下文构建 |
| **CST** | Concrete Syntax Tree：包含所有语法标记（标点、空白）的完整解析树 |
| **AST** | Abstract Syntax Tree：去除语法噪声的抽象语法树 |
| **SymbolId** | 全局唯一的符号标识符 |
