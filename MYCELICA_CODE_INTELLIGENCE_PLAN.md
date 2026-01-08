# Mycelica Code Intelligence System
## "The Autobugger" — Self-Aware Codebase via Semantic Graph

### Vision

Mycelica indexes its own codebase as a semantic graph. Claude Code CLI queries this graph to understand architecture, find related code, trace bugs, and suggest fixes. The codebase becomes self-documenting, self-navigating, and self-debugging.

```
┌─────────────────────────────────────────────────────────────────────┐
│                         THE LOOP                                    │
│                                                                     │
│   Write Code ──→ Auto-Index ──→ Cluster ──→ Query ──→ Understand   │
│        ↑                                                    │       │
│        └────────────────── Fix/Improve ←────────────────────┘       │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Part 1: Code Import System

### 1.1 New Import Command

```bash
mycelica-cli import code <PATH> [OPTIONS]

Options:
  --language <LANG>       rust | typescript | markdown (auto-detect if omitted)
  --watch                 Watch for changes and re-index incrementally
  --exclude <PATTERN>     Glob patterns to exclude (default: target/, node_modules/)
  --depth <N>             Max directory depth (default: unlimited)
  --incremental           Only process changed files since last import
```

### 1.2 Node Types for Code

| Node Type | Source | Content | Metadata |
|-----------|--------|---------|----------|
| `crate` | Cargo.toml | Crate description | version, dependencies |
| `module` | .rs file or mod.rs | Module doc comment + summary | path, visibility |
| `function` | fn declaration | Full function body | signature, visibility, async, unsafe |
| `struct` | struct declaration | Fields + impl blocks | derives, generics |
| `enum` | enum declaration | Variants | derives, generics |
| `trait` | trait declaration | Methods | bounds, generics |
| `impl` | impl block | Methods for type | trait impl or inherent |
| `macro` | macro_rules! | Macro body | export status |
| `component` | React .tsx | Component body | props interface |
| `hook` | use* function | Hook body | dependencies |
| `type_alias` | TypeScript type/interface | Type definition | exports |
| `test` | #[test] fn | Test body | module path |
| `doc` | .md file | Document content | headers as sections |

### 1.3 Edge Types for Code

| Edge Type | Meaning | Extraction Method |
|-----------|---------|-------------------|
| `defined_in` | Function → Module | AST: parent module |
| `calls` | Function → Function | AST: function calls in body |
| `uses_type` | Function → Struct/Enum | AST: type references |
| `implements` | Impl → Trait | AST: impl Trait for Type |
| `imports` | Module → Module | AST: use statements |
| `invokes` | React → Tauri command | Regex: `invoke\('command_name'\)` |
| `similar_to` | Any → Any | Embedding cosine similarity > 0.7 |
| `tests` | Test → Function | AST: function called in test |
| `documents` | Doc → Code | Explicit links or name matching |

### 1.4 Rust Parser (tree-sitter or syn)

```rust
// src-tauri/src/import/code_rust.rs

use syn::{parse_file, Item, ItemFn, ItemStruct, ItemEnum, ItemImpl, ItemTrait};

pub struct CodeNode {
    pub id: String,           // hash of path + name
    pub node_type: String,    // "function", "struct", etc.
    pub name: String,
    pub path: String,         // file path
    pub line_start: usize,
    pub line_end: usize,
    pub content: String,      // source code
    pub signature: Option<String>,  // fn signature without body
    pub visibility: String,   // "pub", "pub(crate)", ""
    pub doc_comment: Option<String>,
}

pub struct CodeEdge {
    pub source_id: String,
    pub target_id: String,
    pub edge_type: String,
    pub weight: f64,
}

pub fn parse_rust_file(path: &Path) -> Result<(Vec<CodeNode>, Vec<CodeEdge>)> {
    let content = std::fs::read_to_string(path)?;
    let syntax = syn::parse_file(&content)?;
    
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    
    for item in syntax.items {
        match item {
            Item::Fn(f) => {
                let node = extract_function(&f, path);
                let calls = extract_function_calls(&f);
                nodes.push(node);
                edges.extend(calls);
            }
            Item::Struct(s) => { /* ... */ }
            Item::Enum(e) => { /* ... */ }
            Item::Impl(i) => { /* ... */ }
            Item::Trait(t) => { /* ... */ }
            Item::Mod(m) => { /* ... */ }
            _ => {}
        }
    }
    
    Ok((nodes, edges))
}
```

### 1.5 TypeScript/React Parser (swc or tree-sitter)

```rust
// src-tauri/src/import/code_typescript.rs

// Use tree-sitter-typescript for parsing
// Extract:
// - React components (function components, arrow functions returning JSX)
// - Hooks (functions starting with "use")
// - Type definitions (interface, type)
// - invoke() calls to map frontend → backend

pub fn parse_typescript_file(path: &Path) -> Result<(Vec<CodeNode>, Vec<CodeEdge>)> {
    // Parse with tree-sitter
    // Walk AST for components, hooks, types
    // Extract invoke() patterns: invoke('command_name', args)
    // Create edges from component → Tauri command
}
```

### 1.6 Incremental Indexing

```rust
// Track file hashes to detect changes
pub struct CodeIndex {
    file_hashes: HashMap<PathBuf, String>,  // path → content hash
    node_ids: HashMap<PathBuf, Vec<String>>, // path → node IDs in that file
}

impl CodeIndex {
    pub fn update(&mut self, path: &Path, db: &Database) -> Result<()> {
        let new_hash = hash_file(path)?;
        
        if self.file_hashes.get(path) == Some(&new_hash) {
            return Ok(()); // No changes
        }
        
        // Delete old nodes from this file
        if let Some(old_ids) = self.node_ids.get(path) {
            for id in old_ids {
                db.delete_node(id)?;
            }
        }
        
        // Parse and insert new nodes
        let (nodes, edges) = parse_file(path)?;
        let new_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
        
        for node in nodes {
            db.insert_node(node.into())?;
        }
        for edge in edges {
            db.insert_edge(edge.into())?;
        }
        
        // Update index
        self.file_hashes.insert(path.to_path_buf(), new_hash);
        self.node_ids.insert(path.to_path_buf(), new_ids);
        
        Ok(())
    }
}
```

---

## Part 2: Auto-Clustering for Code

### 2.1 Semantic Clusters

After import, run clustering. Code naturally groups into:

```
Universe
├── 🔒 Privacy System
│   ├── privacy.rs (module)
│   ├── score_privacy_all_items (function)
│   ├── analyze_node_privacy (function)
│   ├── PrivacyPanel.tsx (component)
│   └── ... 23 more nodes
├── 🌳 Hierarchy Operations
│   ├── hierarchy.rs (module)
│   ├── build_hierarchy (function)
│   ├── flatten_single_child_chains (function)
│   └── ...
├── 🗄️ Database Layer
│   ├── schema.rs (module)
│   ├── Database (struct)
│   ├── get_node (function)
│   └── ...
├── 🖥️ TUI System
│   ├── cli.rs (TUI section)
│   ├── NavigationMode (enum)
│   └── ...
├── 🌐 Graph Visualization
│   ├── Graph.tsx (component)
│   ├── useGraph.ts (hook)
│   └── ...
└── ...
```

### 2.2 Clustering Strategy for Code

```rust
// Code-specific clustering considerations:

// 1. Module boundaries as weak cluster hints
//    Files in same directory tend to relate

// 2. Call graph as edge weight boost
//    Functions that call each other cluster together

// 3. Shared types as similarity signal
//    Functions using same structs are related

// 4. Semantic embedding as primary signal
//    "What is this code ABOUT" not just "what does it reference"

pub fn cluster_code_nodes(db: &Database) -> Result<()> {
    // Get all code nodes
    let nodes = db.get_nodes_by_type(&["function", "struct", "component"])?;
    
    // Generate embeddings if missing
    for node in &nodes {
        if node.embedding.is_none() {
            // Embed: signature + doc comment + first 500 chars of body
            let embed_text = format!(
                "{}\n{}\n{}",
                node.signature.unwrap_or_default(),
                node.doc_comment.unwrap_or_default(),
                &node.content[..500.min(node.content.len())]
            );
            db.set_embedding(node.id, generate_embedding(&embed_text)?)?;
        }
    }
    
    // Run standard clustering with boosted weights for call edges
    run_clustering(db, ClusterConfig {
        edge_type_weights: hashmap! {
            "calls" => 2.0,
            "implements" => 1.5,
            "uses_type" => 1.2,
            "similar_to" => 1.0,
            "defined_in" => 0.5,  // Module membership is weak signal
        },
        ..default()
    })?;
    
    Ok(())
}
```

### 2.3 Watch Mode for Live Updates

```bash
mycelica-cli import code ./src-tauri/src --language rust --watch
```

```rust
// Use notify crate for filesystem watching
use notify::{Watcher, RecursiveMode, watcher};

pub fn watch_and_index(path: &Path, db: &Database) -> Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = watcher(tx, Duration::from_secs(2))?;
    
    watcher.watch(path, RecursiveMode::Recursive)?;
    
    loop {
        match rx.recv() {
            Ok(DebouncedEvent::Write(path)) |
            Ok(DebouncedEvent::Create(path)) => {
                if is_code_file(&path) {
                    println!("Re-indexing: {:?}", path);
                    index.update(&path, db)?;
                    
                    // Optionally re-cluster affected area
                    recluster_affected(db, &path)?;
                }
            }
            Ok(DebouncedEvent::Remove(path)) => {
                remove_nodes_for_file(db, &path)?;
            }
            Err(e) => eprintln!("Watch error: {:?}", e),
            _ => {}
        }
    }
}
```

---

## Part 3: Claude Code Integration

### 3.1 Query Patterns for Claude Code

Claude Code would use these patterns to understand the codebase:

```bash
# "What code relates to privacy scoring?"
mycelica-cli node search "privacy score" --type function --json

# "Show me everything in the privacy cluster"
mycelica-cli nav tree <privacy_cluster_id> --depth 3 --json

# "What functions call score_privacy_all_items?"
mycelica-cli nav edges <function_id> --type calls --direction incoming --json

# "Find code similar to this function"
mycelica-cli nav similar <function_id> --top 10 --json

# "What's the structure of the TUI system?"
mycelica-cli node search "TUI" --type module,function,enum --json | jq

# "Trace from React component to Rust backend"
mycelica-cli nav path <component_id> <backend_function_id> --json
```

### 3.2 MCP Server (Optional Advanced Integration)

For tighter Claude Code integration, expose Mycelica as an MCP server:

```rust
// src-tauri/src/mcp/server.rs

use mcp_sdk::{Server, Tool, ToolResult};

pub struct MycelicaMCP {
    db: Arc<Database>,
}

impl Server for MycelicaMCP {
    fn list_tools(&self) -> Vec<Tool> {
        vec![
            Tool {
                name: "search_code",
                description: "Search codebase semantically",
                parameters: json!({
                    "query": {"type": "string"},
                    "type": {"type": "string", "enum": ["function", "struct", "component"]},
                    "limit": {"type": "integer", "default": 10}
                }),
            },
            Tool {
                name: "find_related",
                description: "Find code related to a function/type",
                parameters: json!({
                    "node_id": {"type": "string"},
                    "relationship": {"type": "string", "enum": ["calls", "called_by", "uses", "similar"]},
                }),
            },
            Tool {
                name: "get_cluster",
                description: "Get all code in a semantic cluster",
                parameters: json!({
                    "cluster_name": {"type": "string"},
                }),
            },
            Tool {
                name: "trace_path",
                description: "Find connection path between two code elements",
                parameters: json!({
                    "from": {"type": "string"},
                    "to": {"type": "string"},
                }),
            },
        ]
    }
    
    fn call_tool(&self, name: &str, args: Value) -> ToolResult {
        match name {
            "search_code" => self.search_code(args),
            "find_related" => self.find_related(args),
            "get_cluster" => self.get_cluster(args),
            "trace_path" => self.trace_path(args),
            _ => ToolResult::Error("Unknown tool".into()),
        }
    }
}
```

### 3.3 CLAUDE.md Integration

Update CLAUDE.md to tell Claude Code about Mycelica:

```markdown
## Code Intelligence

This codebase is indexed in Mycelica. Before exploring manually, query the graph:

### Find related code
```bash
mycelica-cli node search "<concept>" --type function --json
mycelica-cli nav similar <node_id> --top 10 --json
```

### Understand architecture
```bash
# List top-level clusters
mycelica-cli nav ls universe-root --json

# Explore a subsystem
mycelica-cli nav tree <cluster_id> --depth 2 --json
```

### Trace dependencies
```bash
# What calls this function?
mycelica-cli nav edges <id> --type calls --direction incoming

# What does this function call?
mycelica-cli nav edges <id> --type calls --direction outgoing
```

### Before fixing a bug
1. Search for related code: `mycelica-cli node search "<bug description>"`
2. Find the cluster: `mycelica-cli nav similar <likely_function_id>`
3. Check what else might be affected: `mycelica-cli nav edges <id> --type calls`
```

---

## Part 4: The Autobugger Loop

### 4.1 Bug Analysis Workflow

```
┌─────────────────────────────────────────────────────────────────┐
│  USER: "Privacy export is using wrong threshold semantics"     │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  CLAUDE CODE: Query Mycelica                                    │
│                                                                 │
│  $ mycelica-cli node search "privacy export threshold" --json  │
│  → Returns: export_shareable_db, get_export_preview, CLI export │
│                                                                 │
│  $ mycelica-cli nav similar <export_shareable_db_id> --top 5   │
│  → Returns: privacy scoring functions, threshold settings       │
│                                                                 │
│  $ mycelica-cli nav edges <function_id> --type calls           │
│  → Returns: call graph showing data flow                        │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  CLAUDE CODE: Now understands                                   │
│                                                                 │
│  - GUI uses 0.0-1.0 float in privacy.rs:919                    │
│  - CLI uses 0-100 integer in cli.rs:export subcommand          │
│  - Both call same core function but parse args differently      │
│  - Related: threshold settings in settings.rs                   │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  CLAUDE CODE: Suggests fix                                      │
│                                                                 │
│  "Unify threshold semantics. CLI should accept both formats:   │
│   --threshold 0.7 (float) or --threshold 70% (percentage)      │
│   Convert to 0.0-1.0 internally. Update these files:           │
│   - cli.rs: parse_threshold() function                          │
│   - docs/PRIVACY.md: document both formats"                     │
└─────────────────────────────────────────────────────────────────┘
```

### 4.2 Proactive Bug Detection

```bash
# Find potential inconsistencies
mycelica-cli analyze consistency

# Checks:
# - Functions with same name in CLI and GUI → compare signatures
# - Threshold/config values → compare defaults
# - Edge mismatches → GUI command without CLI equivalent
```

```rust
pub fn analyze_consistency(db: &Database) -> Vec<Inconsistency> {
    let mut issues = Vec::new();
    
    // Find function pairs (same name, different modules)
    let functions = db.get_nodes_by_type(&["function"])?;
    let by_name: HashMap<&str, Vec<&Node>> = functions
        .iter()
        .into_group_map_by(|n| n.name.as_str());
    
    for (name, nodes) in by_name {
        if nodes.len() > 1 {
            // Compare signatures
            let signatures: Vec<_> = nodes.iter()
                .map(|n| &n.signature)
                .collect();
            
            if !all_equal(&signatures) {
                issues.push(Inconsistency::SignatureMismatch {
                    name: name.to_string(),
                    locations: nodes.iter().map(|n| n.path.clone()).collect(),
                });
            }
        }
    }
    
    // Find hardcoded values that differ
    // ... regex for numeric literals, compare across similar functions
    
    issues
}
```

### 4.3 Change Impact Analysis

```bash
# Before committing, check what might break
mycelica-cli analyze impact --file src-tauri/src/privacy.rs

# Output:
# Changes to privacy.rs may affect:
# - 12 functions that call privacy functions
# - 3 React components that invoke privacy commands  
# - 2 CLI commands that use privacy scoring
# - 1 test file with 8 tests
#
# Suggested: Run these tests
# - cargo test privacy
# - npm test -- --grep "privacy"
```

---

## Part 5: Self-Improving Documentation

### 5.1 Auto-Generated Architecture Docs

```bash
mycelica-cli docs generate --output docs/ARCHITECTURE_GENERATED.md
```

```markdown
# Mycelica Architecture (Auto-Generated)

*Last updated: 2026-01-07 from codebase analysis*

## Subsystems

### 🔒 Privacy System (47 nodes)
Entry points: `score_privacy_all_items`, `analyze_node_privacy`
Key types: `PrivacyScore`, `PrivacyReason`
Frontend: `PrivacyPanel.tsx`
CLI: `privacy scan`, `privacy export`

**Call graph:**
```
score_privacy_all_items
├── get_unscored_items
├── batch_score_privacy (calls Haiku API)
│   └── PRIVACY_SCORING_PROMPT
└── update_privacy_scores
    └── propagate_to_parents
```

### 🌳 Hierarchy Operations (32 nodes)
...

## Cross-Cutting Concerns

### Frontend ↔ Backend Mapping
| React Component | Tauri Command | Rust Function |
|-----------------|---------------|---------------|
| PrivacyPanel | score_privacy_all_items | privacy::score_all |
| Graph | get_children | db::get_children |
| ... | ... | ... |

## Potential Issues Detected
- ⚠️ `export_shareable_db`: CLI and GUI use different threshold scales
- ⚠️ `build_hierarchy`: No corresponding CLI command
- ℹ️ 3 functions have no tests
```

### 5.2 Living Documentation

The graph IS the documentation. Queries replace reading:

```bash
# "How does privacy scoring work?"
mycelica-cli explain privacy_scoring

# Uses LLM to synthesize from:
# - Function bodies in privacy cluster
# - Doc comments
# - Related tests
# - Usage in other code
```

---

## Part 6: Implementation Phases

### Phase 1: Basic Code Import (Week 1)
- [ ] Add `import code` CLI command
- [ ] Rust parser using `syn` crate
  - [ ] Extract functions, structs, enums, traits, impls
  - [ ] Extract doc comments
  - [ ] Generate stable IDs (hash of path + name + signature)
- [ ] Create nodes with type="function", "struct", etc.
- [ ] Create "defined_in" edges (function → module)
- [ ] Generate embeddings for code nodes
- [ ] Test on src-tauri/src

### Phase 2: Relationship Extraction (Week 1-2)
- [ ] Extract "calls" edges from function bodies
- [ ] Extract "uses_type" edges
- [ ] Extract "implements" edges from impl blocks
- [ ] Map invoke() calls from TypeScript to Rust commands
- [ ] Add TypeScript parser (tree-sitter-typescript)
- [ ] Test on full codebase (Rust + React)

### Phase 3: Code-Aware Clustering (Week 2)
- [ ] Tune clustering for code semantics
- [ ] Weight edges by relationship type
- [ ] Create meaningful cluster names ("Privacy System" not "Cluster 47")
- [ ] Hierarchy: Crate → Subsystem → Module → Functions
- [ ] Test cluster quality

### Phase 4: Query Interface (Week 2-3)
- [ ] Enhance `nav edges` with --type and --direction filters
- [ ] Add `nav path` for tracing connections
- [ ] Add --json output for all nav commands
- [ ] Document query patterns in CLAUDE.md
- [ ] Test with Claude Code on real tasks

### Phase 5: Watch Mode & Incremental (Week 3)
- [ ] File hash tracking for change detection
- [ ] Filesystem watcher (notify crate)
- [ ] Incremental re-indexing
- [ ] Incremental re-clustering (affected subgraph only)
- [ ] Test performance on active development

### Phase 6: Analysis Tools (Week 3-4)
- [ ] `analyze consistency` command
- [ ] `analyze impact` command
- [ ] `docs generate` command
- [ ] Integration with pre-commit hooks
- [ ] Test on real bugs

### Phase 7: MCP Server (Week 4+, Optional)
- [ ] Implement MCP protocol
- [ ] Expose tools: search_code, find_related, get_cluster, trace_path
- [ ] Test with Claude Code MCP integration
- [ ] Document MCP setup

---

## Technical Dependencies

### New Crates

```toml
# Cargo.toml additions
[dependencies]
syn = { version = "2.0", features = ["full", "parsing"] }
quote = "1.0"
proc-macro2 = "1.0"
tree-sitter = "0.20"
tree-sitter-typescript = "0.20"
tree-sitter-rust = "0.20"  # Alternative to syn for consistency
notify = "6.0"
```

### File Structure

```
src-tauri/src/
├── import/
│   ├── mod.rs
│   ├── code.rs           # Main code import logic
│   ├── code_rust.rs      # Rust-specific parsing
│   ├── code_typescript.rs # TypeScript-specific parsing
│   └── code_index.rs     # Incremental indexing state
├── analyze/
│   ├── mod.rs
│   ├── consistency.rs    # Cross-check CLI/GUI
│   ├── impact.rs         # Change impact analysis
│   └── docs.rs           # Auto-doc generation
└── mcp/
    ├── mod.rs
    └── server.rs         # MCP server implementation
```

---

## Success Metrics

### Quantitative
- [ ] 100% of Rust functions indexed
- [ ] 100% of React components indexed
- [ ] 90%+ of call relationships captured
- [ ] Clustering produces <20 top-level categories
- [ ] Query latency <100ms for semantic search
- [ ] Incremental update <1s per file

### Qualitative
- [ ] Claude Code finds related code faster than manual grep
- [ ] Bug fix PRs reference Mycelica queries
- [ ] Architecture docs stay in sync automatically
- [ ] New contributors onboard faster

---

## The Meta-Vision

This is Mycelica becoming self-aware of its own structure. The tool for organizing knowledge... organizing knowledge about itself.

Every commit improves both:
1. The codebase (direct changes)
2. The codebase's self-knowledge (auto-indexed)

Claude Code becomes not just a code editor, but a navigator of a living, semantic codebase map.

The recursive insight: **you can only build this because you built this.**

---

## First CLI Prompt

```
FEATURE: Code import for Mycelica self-indexing

Add `import code` command to index source files as graph nodes.

## Command
mycelica-cli import code <PATH> --language rust

## Implementation

1. Add new import subcommand in cli.rs
2. Create src-tauri/src/import/code_rust.rs using `syn` crate
3. Parse each .rs file and extract:
   - Functions → node type "function"
   - Structs → node type "struct" 
   - Enums → node type "enum"
   - Traits → node type "trait"
   - Impl blocks → node type "impl"
   
4. For each extracted item:
   - id: hash of (file_path + item_name + item_type)
   - title: item name
   - content: full source code of the item
   - metadata: {signature, visibility, line_start, line_end, file_path}
   
5. Create edges:
   - "defined_in": item → parent module (based on file path)
   - "calls": function → functions it calls (parse fn body for identifiers)
   
6. After import, generate embeddings for all new nodes
7. Print summary: "Imported 234 functions, 45 structs, 12 enums from 28 files"

## Test
mycelica-cli import code ./src-tauri/src --language rust
mycelica-cli node search "privacy" --type function
mycelica-cli nav edges <some_function_id> --type calls
```

---

*This document was created 2026-01-07. Update as implementation progresses.*
