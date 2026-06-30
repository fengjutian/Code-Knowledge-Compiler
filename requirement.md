下面这份文档，我会按照**可以直接开源**、**可以直接开发**的标准来设计。

我建议产品名称先使用：

> **CKC (Code Knowledge Compiler)**

副标题：

> **Compile Code into Queryable Knowledge.**

---

# 一、产品定位（Product Vision）

## 1.1 愿景

CKC 是一个使用 **Rust** 开发的代码知识编译器（Code Knowledge Compiler）。

它不是一个 RAG 工具。

也不是一个 Code Wiki。

而是一个**代码理解基础设施（Infrastructure）**。

CKC 将整个代码仓库编译成一个可查询的知识中间表示（Knowledge IR），供 AI、IDE、Code Review、Wiki、Agent 等系统使用。

一句话：

> **Compile your repository into a queryable knowledge graph.**

---

# 二、为什么需要 CKC

目前 AI 阅读代码主要有三种方式。

## 第一代

直接把代码发送给 LLM。

```
Repository

↓

LLM
```

问题：

* Token 消耗巨大
* 上下文限制
* 重复理解
* 成本高

---

## 第二代

RAG

```
Repository

↓

Chunk

↓

Embedding

↓

Vector Search

↓

LLM
```

问题：

Chunk 只是文本。

没有：

* 调用关系
* 生命周期
* 架构
* 模块职责

LLM 每次仍然需要重新理解。

---

## 第三代（CKC）

```
Repository

↓

Compiler

↓

Knowledge IR

↓

Query Runtime

↓

LLM
```

Repository 只需要编译一次。

之后所有 AI 都查询 Knowledge IR。

---

# 三、核心理念

CKC 不检索代码。

CKC 查询知识。

即：

```
Code

↓

Semantic

↓

Knowledge

↓

Reasoning
```

---

# 四、系统总体架构

```
                     Repository
                          │
                          ▼
              ┌────────────────────┐
              │ Incremental Scanner│
              └────────────────────┘
                          │
                          ▼
              ┌────────────────────┐
              │ Language Parser    │
              └────────────────────┘
                          │
                          ▼
                 AST / Symbol Tree
                          │
                          ▼
              ┌────────────────────┐
              │ Semantic Compiler  │
              └────────────────────┘
                          │
                          ▼
                  Knowledge IR
                          │
      ┌──────────────┬───────────────┐
      ▼              ▼               ▼
 Graph Store    Vector Store    Metadata Store
      │              │               │
      └──────────────┴───────────────┘
                     │
                     ▼
                Query Runtime
                     │
                     ▼
                  AI / IDE
```

---

# 五、Knowledge IR

这是整个系统最核心的数据结构。

Knowledge IR 不保存代码。

Knowledge IR 保存：

## Node

例如：

```
Function

Class

Trait

Module

File

Service

API

Database Table
```

---

## Edge

例如：

```
Calls

Uses

Implements

Imports

DependsOn

Contains

Creates

Reads

Writes
```

---

## Semantic

例如：

```
Purpose

Summary

Responsibility

Business Capability

Design Pattern

Risk

Complexity
```

---

## Embedding

用于：

Semantic Search

---

## Metadata

例如：

```
Language

Owner

Last Update

Commit

Hash

Coverage
```

---

# 六、编译流程

```
Repository

↓

Scanner

↓

Parser

↓

AST

↓

IR Builder

↓

Semantic Compiler

↓

Embedding Compiler

↓

Knowledge Graph

↓

Persist
```

整个过程类似 Rust Compiler。

---

# 七、增量编译

每个节点维护 Hash。

```
File Hash

↓

AST Hash

↓

Semantic Hash

↓

Embedding Hash
```

任何一层没有变化：

直接复用。

因此大型仓库更新速度非常快。

---

# 八、Query Runtime

Runtime 不读取 Repository。

只读取 IR。

```
User Query

↓

Intent Parser

↓

Graph Query

↓

Vector Query

↓

Knowledge Merge

↓

Context Builder

↓

LLM
```

---

# 九、支持的查询

## Architecture

例如：

```
整个订单系统如何工作？
```

---

## Call Graph

例如：

```
谁调用了 PaymentService？
```

---

## Dependency

例如：

```
Redis 删除会影响哪些模块？
```

---

## Business Flow

例如：

```
订单创建流程
```

---

## Root Cause

例如：

```
库存为什么可能扣减失败？
```

---

## Impact Analysis

例如：

```
修改 UserService 会影响哪些 API？
```

---

## Security

例如：

```
有哪些接口没有鉴权？
```

---

## Refactor

例如：

```
哪些 Service 耦合度最高？
```

---

# 十、Rust Workspace

```
ckc/

crates/

    ckc-core
    ckc-parser
    ckc-ir
    ckc-indexer
    ckc-semantic
    ckc-graph
    ckc-storage
    ckc-vector
    ckc-query
    ckc-runtime
    ckc-llm
    ckc-cli
```

---

# 十一、公共 Trait

整个系统完全插件化。

Parser

```
Repository
↓

Node
```

Semantic Compiler

```
Node

↓

Semantic Node
```

Embedding

```
Text

↓

Vector
```

Graph

```
Node

↓

Edge
```

Storage

```
Save

Load

Update
```

LLM

```
Prompt

↓

Summary
```

---

# 十二、支持语言

第一阶段

* Rust
* Python
* TypeScript

第二阶段

* Java
* Go
* C#

第三阶段

任何 Tree-sitter 支持的语言。

---

# 十三、产品路线图

## Phase 1（MVP）

* Repository Scanner
* Rust Parser
* Knowledge IR
* SQLite
* Qdrant
* Graph Query
* CLI

---

## Phase 2

* 增量编译
* 多语言
* API
* LLM Compiler
* Semantic Cache

---

## Phase 3

* IDE SDK
* GitHub App
* VS Code Extension
* AI Agent SDK
* MCP Server

---

## Phase 4

* 企业级 Code Intelligence Platform
* 多仓库知识融合
* 架构演化分析
* 自动设计文档生成
* 自动测试影响分析
* 自动代码审查

# 十四、我建议增加一个“编译器”分层（这是产品的差异化）

如果想让 CKC 与现有 RAG 或 Code Wiki 产品形成明显区别，可以将内部流程明确设计为四级编译器：

```
Repository
      │
      ▼
① Syntax Compiler
(AST、符号、依赖)

      │
      ▼
② Semantic Compiler
(职责、业务能力、设计模式)

      │
      ▼
③ Knowledge Compiler
(知识图谱、Embedding、索引)

      │
      ▼
④ Query Compiler
(自然语言 → 图查询 + 向量检索 + 上下文构建)
```

这样，**Compiler** 就成为整个产品的核心概念，而 **RAG** 只是 Query Compiler 中的一个执行策略，而不是产品本身。这会让产品定位更清晰，也更容易扩展到架构分析、影响分析、自动文档、Agent 等更多能力。
