# Diagnostic Report Import and Comparison

Issue #353 is delivered in phases. The first phase defines the local parser and
comparison contract; Diagnostics navigation and presentation follow after the
Performance/Runs shell from #352 is stable.

## Supported reports

The importer accepts only these JSON contracts:

- legacy Performance Lab `BenchmarkReport` objects without `reportVersion`;
- Performance Lab `BenchmarkReport` schema version 2;
- `murmur-eval` report version 1 with fixture version 1.

Unknown versions, unknown fields, missing or incorrectly typed fields,
non-finite or negative measurements, duplicate model/fixture/case IDs, malformed
JSON, and inconsistent evaluation summary counts fail closed. Benchmark and
evaluation reports are recognized separately and are never interpreted as one
another.

Imports are limited to 8 MiB. The parser also caps benchmark models, fixtures
per model, evaluation cases, stages per case, and string collections. The
future file picker must check the file size before reading it; the parser
rechecks both the declared byte count and decoded UTF-8 size before parsing.

Import errors use stable codes and fixed messages. They never include the
selected path, filename, JSON contents, or a rejected field value.

## In-memory representation and privacy

Normalized imports are session-only. The parser has no storage or telemetry
dependency and never receives a source path. Its normalized output includes the
metadata and numeric/categorical measurements required for comparison, but
deliberately drops:

- benchmark reference, transcript, and delivered transcript text;
- evaluation expected, actual, delivered, and per-stage text;
- evaluation failure strings and provenance/deletion text;
- evaluator bundle IDs, matched profile names, and audio paths.

Evaluation imports always carry a warning that the selected source report may
contain curated fixture transcripts and stage text. Clearing an import in the
future UI will discard only in-session state and will not change or delete the
source file.

## Compatibility gate

Compatibility is evaluated before deltas or recommendations are exposed.
Findings are either blockers or warnings.

Benchmark comparison is blocked when schema, preset, measured iteration count,
corpus identity/counts, VAD threshold, execution path, transform profile,
percentile method, model/shared-initialization order, or exact
model/backend/accelerator sets differ. Failed/incomplete model results and
different per-model fixture sets also block comparison. Legacy reports remain readable, but
their missing environment/corpus/execution metadata prevents proving a
like-for-like comparison, so they cannot produce deltas or recommendations.

Evaluation comparison is blocked when report/fixture schema, deterministic
versus hardware tier, fixture ID set, per-case model/backend/accelerator, stage
sequence, fixture-only mode, final-only behavior, or incremental-completion
semantics differ. Cases marked passed must contain complete recognition,
transformation, and delivery measurements. Failed, skipped, or incomplete case
sets may still be imported for inspection, but cannot produce deltas or
recommendations.

Machine/OS metadata and Murmur app-version differences are visible warnings.
They still permit side-by-side deltas, but disable recommendation eligibility.
Any blocker suppresses all deltas, preventing incompatible metrics from
entering a ranking.

## Delta semantics

For a compatible metric:

```text
absolute delta = candidate - baseline
percentage delta = absolute delta / abs(baseline) * 100
```

Percentage delta is unavailable when the baseline is zero. Missing metrics stay
unavailable rather than becoming zero. Every metric declares its unit and
whether lower or higher values are preferable; the comparison core does not
round values or synthesize a cross-report winner.
