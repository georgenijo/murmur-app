# Omen Codebase Research: Adding the `ctx` Command

Research compiled from 3 parallel sub-agents investigating project structure, analyzer APIs, and semantic search internals.

---

## 1. Project Architecture

Omen is a multi-language code analysis CLI built in Rust. It uses **tree-sitter** for parsing 13 languages and **clap 4.5** (derive macros) for CLI.

### Module Layout

```
src/
  main.rs          — Entry point, command dispatch, handler functions
  cli/mod.rs       — Command enum, arg structs, OutputFormat enum
  core/
    analyzer.rs    — Analyzer trait (all analyzers implement this)
    mod.rs         — Re-exports: Result, FileSet, Config, etc.
  analyzers/       — 17 analyzer modules (complexity, graph, temporal, etc.)
  semantic/        — TF-IDF search engine (index, search, cache, sync, multi-repo)
  parser/mod.rs    — Tree-sitter wrapper, symbol extraction
  mcp/mod.rs       — JSON-RPC MCP server exposing analyzers as tools
  git/             — Git operations (log, blame, diff)
  output/          — Output formatting (JSON/Markdown/text)
  config/          — TOML config loading
  score/           — Composite health scoring
```

### Data Flow

```
CLI args (clap) → run_with_path() → match Command variant
  → build FileSet + AnalysisContext
  → analyzer.analyze(&ctx) (parallel via rayon)
  → Format output (JSON/Markdown/text)
  → stdout
```

### Core Trait

All analyzers implement `Analyzer` (`src/core/analyzer.rs`):

```rust
pub trait Analyzer: Send + Sync {
    type Output: Serialize + Send;
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn analyze(&self, ctx: &AnalysisContext<'_>) -> Result<Self::Output>;
    fn requires_git(&self) -> bool { false }
    fn configure(&mut self, _config: &Config) -> Result<()> { Ok(()) }
}
```

`AnalysisContext` bundles: root path, FileSet, git path, Config, optional progress callback, optional content source.

---

## 2. How to Add a New Command

### Step-by-step pattern

**Step 1: Define CLI args** (`src/cli/mod.rs`):
```rust
#[derive(Args)]
pub struct CtxArgs {
    /// Natural language task description
    pub query: String,

    /// Max files to return
    #[arg(long, default_value = "20")]
    pub top_k: usize,
}
```

**Step 2: Add command variant** (`src/cli/mod.rs`, `Command` enum):
```rust
/// Generate task-scoped context from a natural language description
Ctx(CtxArgs),
```

**Step 3: Add handler in `main.rs`** (in `run_with_path` match):
```rust
Command::Ctx(args) => {
    run_ctx(path, &config, &args, format)?;
}
```

**Step 4: Implement handler** (`main.rs` or a new module):
```rust
fn run_ctx(path: &PathBuf, config: &Config, args: &CtxArgs, format: Format) -> Result<()> {
    // Orchestrate: semantic search → graph → temporal → hotspot
    // Merge and rank results
    // Format and output
}
```

**Step 5: MCP exposure** (`src/mcp/mod.rs`):
- Add tool definition JSON in `handle_tools_list()` (~line 111-344)
- Add match arm in `handle_tool_call()` (~line 371-398)
- MCP tool names are bare strings (e.g., `"ctx"`)

### Note on existing `context` command

There is already an `omen context` (alias `ctx`) command that generates deep context for LLM consumption. The new `ctx` command either needs a different name or should replace/extend this existing command. **This is an open question for the maintainer.**

### Files to modify

| File | Change |
|------|--------|
| `src/cli/mod.rs` | Add `CtxArgs` struct + `Command::Ctx` variant |
| `src/main.rs` | Add match arm + handler function |
| `src/mcp/mod.rs` | Add tool definition + dispatch |
| `src/analyzers/` (optional) | New module if it follows the Analyzer trait pattern |

---

## 3. Existing Analyzers We'd Chain

### 3.1 Semantic Search (TF-IDF)

**Location**: `src/semantic/`

**How to invoke programmatically**:
```rust
let search_config = SearchConfig {
    cache_path: Some(path.join(".omen/search.db")),
    max_results: 20,
    min_score: 0.3,
};
let search = SemanticSearch::new(&search_config, path)?;
search.index(config)?;  // Ensure index is up-to-date (incremental)
let output = search.search("fix auth token refresh", Some(20))?;
// output.results: Vec<SearchResult> — ranked by cosine similarity
```

**Key types**:
- `SearchOutput { query, total_symbols, results: Vec<SearchResult> }`
- `SearchResult { file_path, symbol_name, symbol_type, signature, start_line, end_line, score, cyclomatic_complexity, cognitive_complexity }`

**Filtered search** (by complexity, by files):
```rust
search.search_filtered(query, top_k, &SearchFilters { min_score: 0.3, max_complexity: Some(15) })?;
search.search_in_files(query, &["src/auth.rs", "src/token.rs"], top_k)?;
```

**Initialization**: Requires `SemanticSearch::new()` + `.index()`. Uses SQLite cache at `.omen/search.db`.

---

### 3.2 Dependency Graph

**Location**: `src/analyzers/graph.rs`

**How to invoke**:
```rust
let analyzer = graph::Analyzer::default();
let ctx = AnalysisContext::new(&file_set, &config, Some(root));
let analysis = analyzer.analyze(&ctx)?;
// analysis contains: PageRank scores, betweenness centrality, instability, cycles
```

**What it provides for `ctx`**:
- Given seed files from semantic search, find their **dependencies** (imports) and **dependents** (what imports them)
- PageRank scores indicate file importance in the dependency graph
- Instability metric: `out_degree / (in_degree + out_degree)` — 1.0 = highly unstable
- Cycle detection via Tarjan's SCC

**Graph metrics per node**: PageRank, betweenness centrality, instability, in/out degree

**Import resolution**: `FilePathIndex::find_match()` resolves import paths to actual files using multi-strategy matching (exact path, stem, segments, snake_case).

---

### 3.3 Temporal Coupling

**Location**: `src/analyzers/temporal.rs`

**How to invoke**:
```rust
let analyzer = temporal::Analyzer::new()
    .with_days(90)
    .with_min_cochanges(3);
let analysis = analyzer.analyze_repo(root_path)?;
// analysis.couplings: Vec<FileCoupling>
```

**What it provides for `ctx`**:
- Given seed files, find files that **historically change together** with them
- Coupling strength: `cochange_count / max(commits_a, commits_b)` (0-1 scale)
- Strong coupling threshold: >= 0.5
- Filters out "mega commits" (>100 files) to avoid noise

**Key type**: `FileCoupling { file_a, file_b, cochange_count, coupling_strength, commits_a, commits_b }`

**Usage pattern for `ctx`**: After semantic search finds seed files, filter temporal couplings for entries where `file_a` or `file_b` matches any seed file.

---

### 3.4 Hotspot (Churn x Complexity)

**Location**: `src/analyzers/hotspot.rs`

**How to invoke**:
```rust
let analyzer = hotspot::Analyzer::new()
    .with_days(90);
let analysis = analyzer.analyze_project(root_path)?;
// analysis.hotspots: Vec<Hotspot>
```

**What it provides for `ctx`**:
- Identifies files with both high churn AND high complexity — likely trouble spots
- Combined score = `churn_percentile * complexity_percentile`
- Severity levels: Critical (>=0.81), High (>=0.64), Moderate (>=0.36), Low

**Key type**: `Hotspot { path, churn_score, complexity_score, combined_score, severity }`

**Usage pattern for `ctx`**: After gathering candidate files, filter hotspots to flag which files are risky.

---

### 3.5 Repomap (PageRank Symbol Map)

**Location**: `src/analyzers/repomap.rs`

**How to invoke**:
```rust
let analyzer = repomap::Analyzer::new()
    .with_max_symbols(100)
    .with_skip_test_files(true);
let analysis = analyzer.analyze_repo(root_path)?;
// or: analyzer.analyze_with_files(root_path, &file_set)?;  // subset
// analysis.symbols: Vec<RankedSymbol>
```

**What it provides for `ctx`**:
- PageRank-ranked symbols across the codebase (or a subset via FileSet)
- Call graph analysis: in-degree (callers), out-degree (callees)
- Can limit to top N symbols

**Key type**: `RankedSymbol { qualified_name, name, kind, file, line, signature, pagerank_score, in_degree, out_degree, is_exported }`

---

### 3.6 Complexity (per-file/per-function)

**Location**: `src/analyzers/complexity.rs`

**How to invoke**:
```rust
let analyzer = complexity::Analyzer::new();
let file_result = analyzer.analyze_file(path)?;
// or: analyzer.analyze_content(path, content_bytes)?;
// file_result.functions: Vec<FunctionComplexity>
```

**Key type**: `FunctionComplexity { name, start_line, end_line, cyclomatic, cognitive }`

---

### Summary: All analyzers are stateless

| Analyzer | Stateless | Requires Git | Key Input | Key Output |
|----------|-----------|-------------|-----------|------------|
| Semantic Search | No (SQLite cache) | No | query string | Ranked symbols with scores |
| Graph | Yes | No | FileSet | PageRank, deps, cycles |
| Temporal | Yes | Yes | days, min_cochanges | File co-change pairs |
| Hotspot | Yes | Yes | days | Churn x complexity scores |
| Repomap | Yes | No | FileSet | PageRank-ranked symbols |
| Complexity | Yes | No | file path | Per-function metrics |

---

## 4. Semantic Search Internals

### Pipeline: File → Symbols → Chunks → Index → Search

```
1. PARSE: tree-sitter extracts FunctionNode per language
   - Thread-local parser cache (lock-free parallel parsing)
   - FunctionNode: name, signature, lines, is_exported, body_byte_range

2. CHUNK: Long functions split at statement boundaries
   - MAX_CHUNK_CHARS = 500
   - Each chunk carries parent struct/class name
   - Chunk { file_path, symbol_name, parent_name, signature, content, chunk_index, total_chunks }

3. ENRICH: Format for TF-IDF matching
   - Format: "[file_path] Parent::symbol_name\ncode_content"
   - +15% MRR over bare code in benchmarks

4. CACHE: SQLite (.omen/search.db)
   - Schema: symbols table (enriched_text, content_hash, complexity) + files table (file_hash)
   - Staleness detection via Blake3 file hashes

5. SYNC: Incremental re-indexing
   - Detects changed files via hash comparison
   - Removes deleted files from index
   - Parallel parse + index via rayon

6. INDEX: TF-IDF engine (rebuilt from cache on each search)
   - Tokenization: unigrams + bigrams, regex word splitting
   - Sublinear TF: 1 + ln(tf)
   - Smooth IDF: ln(1 + n/(1+df)) + 1
   - Max vocabulary: 10,000 terms by document frequency
   - L2-normalized sparse vectors

7. SEARCH: Cosine similarity ranking
   - Query → tokenize → TF-IDF vector → dot product with all docs
   - Deduplication: keep best-scoring chunk per symbol
   - Multi-repo: combine caches, prefix file paths with repo labels
```

### HyDE (Hypothetical Document Embedding)

HyDE is **not** an ML embedding. It lets you search by providing a code snippet that resembles what you want to find. The snippet is tokenized and matched against the TF-IDF index, same as a natural language query. This is exposed as `semantic_search_hyde` in MCP.

For `ctx`, HyDE could be used to generate hypothetical code from the task description and search for similar real code.

### Multi-repo Search

`multi_repo_search()` in `src/semantic/multi_repo.rs` combines symbol indexes from multiple projects into a single TF-IDF corpus. File paths are prefixed with repo labels (e.g., `[murmur-app] src/auth.rs`).

---

## 5. Gaps & Open Questions

### Naming conflict

There's already an `omen context` command (alias `ctx`) that generates deep context for LLMs. Options:
1. **Extend** the existing `context` command with a `--task` flag
2. **Replace** it entirely with the new task-scoped behavior
3. **Use a different name** (e.g., `omen task`, `omen scope`, `omen focus`)

**Recommendation**: Investigate what the existing `context` command does in detail and decide whether to extend or replace.

### TF-IDF index is rebuilt on every search

The TF-IDF engine is reconstructed from the SQLite cache on each `search()` call. For `ctx`, which chains multiple searches, this means:
- The index sync only needs to happen once
- But the TF-IDF `fit()` runs on every search call
- For large codebases, consider caching the fitted engine across calls within a single `ctx` invocation

### No embedding model

Omen uses TF-IDF, not neural embeddings. This means:
- Lexical match only — "auth" matches "auth" but not "authentication" or "login"
- HyDE partially compensates by letting you search with code snippets
- For `ctx`, the quality of initial seed discovery depends on keyword overlap between the task description and code

### Graph analyzer returns whole-repo analysis

The graph analyzer doesn't have a "query deps for file X" API — it builds the full dependency graph. For `ctx`, we'd need to:
1. Run the full graph analysis
2. Filter the results to extract neighbors of seed files
3. Or build a lighter-weight per-file query function

### Temporal coupling is whole-repo

Same issue as graph — `analyze_repo()` computes all couplings, then we'd filter. For large repos with long history, this could be slow. Consider:
- Limiting `days` parameter (e.g., 90 days)
- Pre-filtering git log to only include commits touching seed files

### Hotspot is whole-repo

`analyze_project()` computes all hotspots. We'd filter to seed files. The `combine_analyses()` method exists but still expects churn/complexity for all files.

### No "importance" ranking across analyzers

Each analyzer produces its own scores (TF-IDF similarity, PageRank, coupling strength, hotspot severity). There's no unified scoring model to merge them. `ctx` would need to design a merging strategy:
- Weighted combination?
- Cascading filter (semantic → expand with graph/temporal → rank with hotspot)?
- Each dimension as a separate column in output?

### Analyzer trait may not fit

The `Analyzer` trait expects `AnalysisContext` with a `FileSet`. The `ctx` command is more of an orchestrator that calls multiple analyzers and merges results. It might not implement `Analyzer` itself — instead it would be a standalone handler function in `main.rs` that internally creates and calls other analyzers.

### MCP tool input schema

The MCP tool for `ctx` needs a clear schema. Key parameters:
- `query` (required): Natural language task description
- `top_k` (optional): Max results
- `path` (optional): Repo root
- `days` (optional): History window for temporal/hotspot

### Test strategy

Per CLAUDE.md, Omen uses TDD. For `ctx`:
- Unit tests for the merging/ranking logic
- Integration tests with a small test repo
- Tests for each stage: seed discovery → expansion → ranking → output
