# Design Doc: `omen context` (ctx) — Task-Scoped Context Assembly

## Summary

`omen context "fix auth token refresh"` takes a natural language task description and returns a ranked list of the files, symbols, and metadata most relevant to that task. It orchestrates Omen's existing analyzers — semantic search for initial seed discovery, dependency graph for structural expansion, temporal coupling for historical co-change patterns, and hotspot analysis for risk flagging — into a single pipeline that produces focused, LLM-consumable context. This replaces the current stub implementation of `omen context`, which parses `--target`, `--symbol`, `--depth`, and `--max-tokens` flags but ignores them all, running only a whole-repo repomap.

## Current State

The `omen context` command (alias `ctx`) exists as a skeleton:

- **CLI args** (`src/cli/mod.rs`, `ContextArgs`): Defines `--target <PATH>`, `--max-tokens <N>` (default 8000), `--symbol <NAME>`, `--depth <N>` (default 2).
- **Handler** (`src/main.rs`, `run_context()`): Instantiates `repomap::Analyzer::default()`, calls `analyzer.analyze(&ctx)`, wraps the result in JSON with the args as metadata. **All four flags are stored in the output JSON but have zero effect on the analysis.**
- **Output**: Full repomap of the entire repository (every symbol, PageRank-ranked).
- **MCP**: Not exposed. The `repomap` MCP tool exists separately.
- **Tests**: Four CLI parsing tests only (`test_context_target`, `test_context_max_tokens`, `test_context_symbol`, `test_context_depth`). No functional tests.
- **Skill** (`plugins/development/skills/context/SKILL.md`): Documents complexity/SATD/risk output that the command does not actually produce.

## Proposed Behavior

### CLI Interface

```
omen context "fix auth token refresh"          # positional query
omen ctx "fix auth token refresh"              # alias
omen context "fix auth token refresh" --target src/auth/  # scope to directory
omen context "fix auth token refresh" --depth 3           # deeper dep traversal
omen context "fix auth token refresh" --max-tokens 4000   # token budget
omen context "fix auth token refresh" --max-files 30      # cap output files
```

Updated `ContextArgs`:

```rust
#[derive(Args)]
pub struct ContextArgs {
    /// Natural language task description
    pub query: String,

    /// Focus analysis on a file or directory
    #[arg(long)]
    pub target: Option<PathBuf>,

    /// Maximum tokens for output (truncates least-relevant entries)
    #[arg(long, default_value = "8000")]
    pub max_tokens: usize,

    /// Depth for dependency graph traversal from seed files
    #[arg(long, default_value = "2")]
    pub depth: usize,

    /// Maximum files in output
    #[arg(long, default_value = "30")]
    pub max_files: usize,

    /// Days of git history for temporal coupling and hotspot analysis
    #[arg(long, default_value = "90")]
    pub days: u32,
}
```

The `--symbol` flag is removed — the query string replaces it. If users want symbol-level focus, they put the symbol name in the query.

### Orchestration Pipeline

```
query string
  │
  ▼
┌─────────────────────┐
│ 1. SEED DISCOVERY   │  Semantic search: query → ranked symbols
│    (semantic search) │  If --target: scope search to target path
└────────┬────────────┘
         │ seed_files: Vec<String>  (unique file paths from top results)
         ▼
┌─────────────────────┐
│ 2. GRAPH EXPANSION  │  Dependency graph: find imports + importers of seed files
│    (graph analyzer)  │  Traverse up to --depth levels
└────────┬────────────┘
         │ expanded_files: HashSet<String>  (seed_files ∪ neighbors)
         ▼
┌─────────────────────┐
│ 3. TEMPORAL EXPAND  │  Temporal coupling: files that co-change with expanded set
│    (temporal)        │  Filter: coupling_strength >= 0.5 AND involves expanded_files
└────────┬────────────┘
         │ candidate_files: HashSet<String>  (expanded ∪ temporally coupled)
         ▼
┌─────────────────────┐
│ 4. ANNOTATE         │  Hotspot + complexity: annotate each candidate with risk
│    (hotspot,         │  Compute per-file scores for ranking
│     complexity)      │
└────────┬────────────┘
         │ annotated: Vec<ContextFile>
         ▼
┌─────────────────────┐
│ 5. RANK & TRUNCATE  │  Weighted scoring, sort, apply --max-files and --max-tokens
│                      │
└────────┬────────────┘
         │
         ▼
       output (JSON / Markdown / text)
```

### Output Format

**JSON** (primary — for LLM consumption):

```json
{
  "query": "fix auth token refresh",
  "target": null,
  "total_candidates": 47,
  "returned": 15,
  "files": [
    {
      "path": "src/auth/service.rs",
      "relevance_score": 0.92,
      "source": "semantic",
      "symbols": [
        {
          "name": "refresh_token",
          "kind": "function",
          "signature": "fn refresh_token(&mut self, old_token: &str) -> Result<String>",
          "start_line": 42,
          "end_line": 89,
          "semantic_score": 0.85,
          "cyclomatic_complexity": 5,
          "cognitive_complexity": 6
        }
      ],
      "annotations": {
        "hotspot_severity": "high",
        "hotspot_score": 0.72,
        "graph_in_degree": 5,
        "graph_out_degree": 3,
        "temporal_couplings": ["src/auth/middleware.rs", "src/auth/types.rs"]
      }
    }
  ],
  "summary": {
    "seed_files": 8,
    "graph_expanded": 12,
    "temporal_expanded": 3,
    "hotspot_critical": 1,
    "hotspot_high": 3
  }
}
```

**Markdown** (for human reading):

```markdown
# Context: "fix auth token refresh"

## Most Relevant Files

### src/auth/service.rs (score: 0.92, source: semantic)
- **refresh_token** (L42-89) — `fn refresh_token(&mut self, old_token: &str) -> Result<String>`
  - Complexity: cyclomatic=5, cognitive=6
  - Hotspot: HIGH (0.72)
  - Coupled with: src/auth/middleware.rs, src/auth/types.rs
  - Graph: 5 callers, 3 callees

...

## Summary
- 8 files from semantic search, 12 from dependency graph, 3 from temporal coupling
- 1 critical hotspot, 3 high hotspots
```

### MCP Tool Schema

Tool name: `context` (matches CLI command name pattern).

```json
{
  "name": "context",
  "description": "Generate task-scoped context from a natural language description. Orchestrates semantic search, dependency graph, temporal coupling, and hotspot analysis to find relevant files and symbols for a given task.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": {
        "description": "Natural language task description",
        "type": "string"
      },
      "path": {
        "description": "Repository root path",
        "type": "string"
      },
      "target": {
        "description": "Focus analysis on a file or directory",
        "type": "string"
      },
      "max_files": {
        "description": "Maximum files in output (default: 30)",
        "type": "integer"
      },
      "depth": {
        "description": "Dependency graph traversal depth (default: 2)",
        "type": "integer"
      },
      "days": {
        "description": "Days of git history for temporal/hotspot analysis (default: 90)",
        "type": "integer"
      }
    },
    "required": ["query"]
  }
}
```

## Pipeline Detail

### Stage 1: Seed Discovery (Semantic Search)

**Analyzer**: `SemanticSearch` (`src/semantic/`)

**API**:
```rust
let search_config = SearchConfig {
    cache_path: Some(root.join(".omen/search.db")),
    max_results: 20,
    min_score: 0.3,
};
let search = SemanticSearch::new(&search_config, root)?;
search.index(config)?;  // Incremental sync

// If --target is set, scope to those files
let output = if let Some(target) = &args.target {
    let target_files: Vec<&str> = /* resolve target to file list */;
    search.search_in_files(&args.query, &target_files, Some(top_k))?
} else {
    search.search(&args.query, Some(top_k))?
};
```

**Input**: `args.query` (natural language string), optional `args.target` (path filter).

**Output**: `SearchOutput { results: Vec<SearchResult> }` where each result has `file_path`, `symbol_name`, `symbol_type`, `signature`, `start_line`, `end_line`, `score` (0-1), `cyclomatic_complexity`, `cognitive_complexity`.

**Filtering**: Take results with `score >= 0.3`. Extract unique `file_path` values as `seed_files`.

**Note**: The `SemanticSearch` instance and its synced index should be created once and reused if we need additional searches later in the pipeline.

### Stage 2: Graph Expansion (Dependency Graph)

**Analyzer**: `graph::Analyzer` (`src/analyzers/graph.rs`)

**API**:
```rust
let analyzer = graph::Analyzer::default();
let ctx = AnalysisContext::new(&file_set, &config, Some(root))
    .with_git_path(git_root);
let graph_analysis = analyzer.analyze(&ctx)?;
```

**Input**: `seed_files` from Stage 1, `args.depth` for traversal limit.

**Processing**: The graph analyzer returns the full dependency graph with PageRank, in/out degree, and import relationships for every file. We post-process:

```rust
fn expand_from_seeds(
    graph: &GraphAnalysis,
    seed_files: &[String],
    max_depth: usize,
) -> HashMap<String, GraphAnnotation> {
    // BFS from seed_files up to max_depth levels
    // At each level, add files that import a seed (dependents)
    //   and files that a seed imports (dependencies)
    // Track depth and direction for each discovered file
    // Return map of file_path -> { in_degree, out_degree, pagerank, depth, direction }
}
```

**Output**: `expanded_files: HashMap<String, GraphAnnotation>` — seed files plus their structural neighbors within `depth` hops. Each entry carries graph metrics (PageRank, in/out degree).

**Filtering**: Only include files that exist in the repo's `FileSet`. Cap expansion at `max_files * 2` to bound work in later stages.

### Stage 3: Temporal Expansion (Temporal Coupling)

**Analyzer**: `temporal::Analyzer` (`src/analyzers/temporal.rs`)

**API**:
```rust
let analyzer = temporal::Analyzer::new()
    .with_days(args.days)
    .with_min_cochanges(3);
let temporal_analysis = analyzer.analyze_repo(root)?;
```

**Input**: `expanded_files` from Stage 2, `args.days` for history window.

**Processing**: The temporal analyzer returns all file couplings. We filter:

```rust
fn find_temporal_neighbors(
    couplings: &[FileCoupling],
    expanded_files: &HashSet<String>,
) -> Vec<(String, f64)> {
    couplings.iter()
        .filter(|c| c.coupling_strength >= 0.5)
        .filter(|c| expanded_files.contains(&c.file_a) || expanded_files.contains(&c.file_b))
        .map(|c| {
            let new_file = if expanded_files.contains(&c.file_a) {
                &c.file_b
            } else {
                &c.file_a
            };
            (new_file.clone(), c.coupling_strength)
        })
        .filter(|(f, _)| !expanded_files.contains(f))
        .collect()
}
```

**Output**: `candidate_files: HashSet<String>` — union of `expanded_files` and temporally coupled files. Each temporal addition carries its `coupling_strength`.

**Filtering**: Only add files with `coupling_strength >= 0.5` (strong coupling threshold from temporal analyzer).

### Stage 4: Annotation (Hotspot + Complexity)

**Analyzer**: `hotspot::Analyzer` (`src/analyzers/hotspot.rs`), `complexity::Analyzer` (`src/analyzers/complexity.rs`)

**API**:
```rust
// Hotspot (whole-repo, then filter)
let hotspot_analyzer = hotspot::Analyzer::new().with_days(args.days);
let hotspot_analysis = hotspot_analyzer.analyze_project(root)?;
let hotspot_map: HashMap<String, &Hotspot> = hotspot_analysis.hotspots.iter()
    .map(|h| (h.path.clone(), h))
    .collect();

// Complexity (per candidate file)
let complexity_analyzer = complexity::Analyzer::new();
for file_path in &candidate_files {
    let abs_path = root.join(file_path);
    if let Ok(result) = complexity_analyzer.analyze_file(&abs_path) {
        // Attach function-level complexity to symbols
    }
}
```

**Input**: `candidate_files` from Stage 3.

**Processing**: For each candidate file, look up its hotspot entry and compute per-function complexity. Attach these as annotations.

**Output**: `Vec<ContextFile>` — each file now has: semantic score (if from Stage 1), graph metrics (if from Stage 2), temporal coupling info (if from Stage 3), hotspot severity + score, and per-symbol complexity.

### Stage 5: Rank & Truncate

**Input**: `Vec<ContextFile>` from Stage 4.

**Processing**: Compute a composite `relevance_score` (see Ranking Strategy below), sort descending, apply `--max-files` limit, then apply `--max-tokens` budget by estimating tokens per entry and dropping tail entries.

**Token estimation**: ~4 tokens per word. For JSON output, estimate tokens from the serialized size of each `ContextFile` entry. Accumulate entries until the budget is reached.

**Output**: Final ranked, truncated `Vec<ContextFile>` serialized to the requested format.

## Ranking Strategy

Each candidate file gets a composite score from multiple signals. Not all files have all signals (a file found via graph expansion won't have a semantic score).

### Scoring Formula

```
relevance_score = w_semantic * semantic_signal
                + w_graph    * graph_signal
                + w_temporal * temporal_signal
                + w_hotspot  * hotspot_signal
```

**Weights** (configurable via `omen.toml`):

| Signal | Weight | Rationale |
|--------|--------|-----------|
| `w_semantic` | 0.50 | Direct textual relevance to the task |
| `w_graph` | 0.25 | Structural proximity to relevant code |
| `w_temporal` | 0.15 | Historical co-change pattern |
| `w_hotspot` | 0.10 | Risk/instability flag |

### Signal Normalization

Each signal is normalized to 0.0–1.0:

- **`semantic_signal`**: Raw TF-IDF cosine similarity score (already 0-1). Files not from semantic search get 0.0.
- **`graph_signal`**: `1.0 / (1.0 + depth)` where `depth` is BFS distance from nearest seed file. Seed files get `1.0 / (1.0 + 0) = 1.0`. Direct neighbors get `0.5`. Files not from graph expansion get 0.0.
- **`temporal_signal`**: `coupling_strength` (already 0-1). Files not from temporal expansion get 0.0.
- **`hotspot_signal`**: `combined_score` from hotspot analyzer (already 0-1, being `churn_percentile * complexity_percentile`). Files without hotspot data get 0.0.

### Source Bonus

Files discovered by multiple stages get a small bonus to reward convergence:

```rust
let source_count = [semantic > 0, graph > 0, temporal > 0]
    .iter().filter(|&&b| b).count();
let convergence_bonus = match source_count {
    3 => 0.10,
    2 => 0.05,
    _ => 0.0,
};
relevance_score += convergence_bonus;
relevance_score = relevance_score.min(1.0);
```

### Tiebreaking

When scores are equal (within 0.001), break ties by:
1. Higher PageRank score (structurally more important)
2. Alphabetical file path (deterministic)

## Performance Considerations

### Whole-Repo Analyzer Overhead

Three analyzers compute whole-repo results that we filter down:
- **Graph**: Builds full import graph. Bounded by number of files and imports. Typically <1s for 500 files.
- **Temporal**: Reads git log. Bounded by `--days`. 90 days of history for 500 files is typically <2s.
- **Hotspot**: Runs churn + complexity. Complexity is parallelized via rayon. Typically <3s for 500 files.

These three should run **in parallel** (rayon or std::thread::scope) since they're independent.

### TF-IDF Rebuild

The TF-IDF engine is rebuilt from the SQLite cache on every `search()` call. Within a single `ctx` invocation, we only call `search()` once, so this is not a concern. The incremental `index()` sync only re-parses changed files (Blake3 hash comparison).

### Parallelism Strategy

```
Sequential:
  1. SemanticSearch::index()  — must complete before search
  2. SemanticSearch::search() — produces seed_files

Parallel (after seeds are known):
  ├── graph::Analyzer::analyze()
  ├── temporal::Analyzer::analyze_repo()
  └── hotspot::Analyzer::analyze_project()

Sequential:
  3. Post-process: expand, filter, annotate, rank
  4. complexity::Analyzer::analyze_file() per candidate (parallelized with rayon)
  5. Format and output
```

### Target Performance

For a 500-file repo:
- Index sync (cached): ~100ms
- Semantic search: ~200ms
- Graph + temporal + hotspot (parallel): ~3s (bounded by hotspot)
- Post-processing + complexity: ~500ms
- **Total: ~4s** (within 5s target)

First run (cold index) adds ~2-5s for full parse + index.

## Files to Modify

| File | Change |
|------|--------|
| `src/cli/mod.rs` | Update `ContextArgs`: add positional `query`, add `--max-files`, `--days`. Remove `--symbol`. |
| `src/main.rs` | Rewrite `run_context()`: replace repomap-only stub with full pipeline orchestration. |
| `src/context/mod.rs` | **New file.** Core pipeline logic: `assemble_context()` function that orchestrates all stages. Keeps `main.rs` handler thin. |
| `src/context/types.rs` | **New file.** Output types: `ContextOutput`, `ContextFile`, `ContextSymbol`, `FileAnnotations`, `ContextSummary`. All `#[derive(Serialize)]`. |
| `src/context/ranking.rs` | **New file.** Scoring formula, signal normalization, convergence bonus, tiebreaking. |
| `src/mcp/mod.rs` | Add `"context"` tool definition in `handle_tools_list()`. Add `"context"` match arm in `handle_tool_call()`. |
| `src/lib.rs` | Add `pub mod context;` declaration. |
| `src/config/mod.rs` | Add optional `[context]` section to config: weights, default days, default max_files. |
| `plugins/development/skills/context/SKILL.md` | Update skill to match new behavior and arguments. |
| `tests/context_integration.rs` | **New file.** Integration tests with a fixture repo. |

## Open Questions

1. **Backward compatibility of `--symbol` removal.** The `--symbol` flag is currently a no-op, but removing it would be a breaking CLI change. Options: (a) remove it since it never worked, (b) keep it as a deprecated alias that appends to the query string. **Recommendation: remove it** — it's a no-op and there are no dependents.

2. **Token estimation accuracy.** The `--max-tokens` truncation uses a rough "4 tokens per word" heuristic. Should we use a proper tokenizer (e.g., tiktoken via a Rust binding)? **Recommendation: start with the heuristic**, add a proper tokenizer later if users report issues.

3. **Config section naming.** Should the `omen.toml` section be `[context]` or `[ctx]`? **Recommendation: `[context]`** — matches the command name, aliases are for CLI only.

4. **Semantic search for non-function symbols.** The current TF-IDF index only contains functions. If the query is about a type, struct, or constant, semantic search may miss it. This is a pre-existing limitation of the search index, not specific to `ctx`. Worth noting but not blocking.

5. **Should `ctx` fall back to repomap when the query is empty?** If invoked as `omen context` with no query, should it behave like the old command (full repomap)? **Recommendation: require the query** — make it a required positional arg. Users wanting the old behavior can use `omen repomap` directly.

6. **Parallel analyzer execution.** The graph, temporal, and hotspot analyzers should run in parallel. Should we use `rayon::scope`, `std::thread::scope`, or keep it sequential for simplicity in v1? **Recommendation: `std::thread::scope`** for the three independent analyzers, rayon for file-level parallelism within each.
