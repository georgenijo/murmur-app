#!/usr/bin/env python3
"""Validate Murmur's CI, trusted release-build, cache, and promotion policy."""

from pathlib import Path
import re
from typing import Optional


ROOT = Path(__file__).resolve().parents[1]
CI_WORKFLOW = ROOT / ".github/workflows/ci.yml"
RELEASE_BUILD_WORKFLOW = ROOT / ".github/workflows/release-build.yml"
RELEASE_REHEARSAL_WORKFLOW = ROOT / ".github/workflows/release-rehearsal.yml"
RELEASE_WORKFLOW = ROOT / ".github/workflows/release.yml"
RELEASE_START_WORKFLOW = ROOT / ".github/workflows/release-start.yml"
OTA_CANARY_WORKFLOW = ROOT / ".github/workflows/ota-canary.yml"
CARGO_TOML = ROOT / "app/src-tauri/Cargo.toml"
NEXTEST_CONFIG = ROOT / "app/src-tauri/.config/nextest.toml"

CI_GUARD = (
    '"${{ github.event_name != \'push\' || '
    "!startsWith(github.event.head_commit.message, 'chore: bump version') }}\""
)
CI_PASS_GUARD = (
    '"${{ always() && (github.event_name != \'push\' || '
    "!startsWith(github.event.head_commit.message, 'chore: bump version')) }}\""
)
RELEASE_BUILD_GUARD = (
    '"${{ github.event_name == \'workflow_dispatch\' || '
    "startsWith(github.event.head_commit.message, 'chore: bump version') }}\""
)
CI_BUILD_JOBS = (
    "typecheck",
    "ui-tests",
    "rust-lint",
    "rust-macos",
)
# The condition each build job runs under: the change filters that select it. A weaker or
# narrower one (say `github` alone) would skip the job on a Rust-only PR, and `ci-pass`
# accepts a skipped job unless its filter selected it.
CI_JOB_CONDITIONS = {
    "typecheck": "needs.changes.outputs.frontend == 'true' || needs.changes.outputs.github == 'true'",
    "ui-tests": "needs.changes.outputs.frontend == 'true' || needs.changes.outputs.github == 'true'",
    "rust-lint": "needs.changes.outputs.rust == 'true' || needs.changes.outputs.github == 'true'",
    "rust-macos": "needs.changes.outputs.rust == 'true' || needs.changes.outputs.github == 'true'",
}
CI_SELECTED_JOB_CHECKS = (
    (
        ("rust", "github"),
        ("rust-lint", "rust-macos"),
        "Rust",
    ),
    (
        ("frontend", "github"),
        ("typecheck", "ui-tests"),
        "frontend",
    ),
)
# The change filters and the outputs that select jobs from them. A misspelt glob
# (`app/src-taurii/**`) or an output forced to `false` would skip the Rust jobs on a Rust-only
# PR, and `ci-pass` accepts a job its filters did not select.
CI_FILTER_PATHS = {
    "frontend": (
        "app/src/**",
        "app/public/**",
        "app/index.html",
        "app/log-viewer.html",
        "app/overlay.html",
        "app/package.json",
        "app/package-lock.json",
        "app/tsconfig*.json",
        "app/vite.config.ts",
        "app/vitest.config.ts",
        "app/vitest.setup.ts",
        "app/visual-fixtures.html",
        "app/visual-tests/**",
        "app/playwright*.ts",
        "app/src-tauri/tauri.conf.json",
        "app/src-tauri/capabilities/**",
    ),
    "rust": (
        "app/src-tauri/**",
        "scripts/configure_ci_rust_library.py",
        "app/src-tauri/Cargo.toml",
        "app/src-tauri/Cargo.lock",
        "Cargo.toml",
        "Cargo.lock",
        "rustfmt.toml",
    ),
    "github": (".github/**",),
    "rust-cache": (
        "scripts/configure_ci_rust_library.py",
        "**/Cargo.toml",
        "**/Cargo.lock",
        "rust-toolchain*",
        ".cargo/**",
        ".github/workflows/ci.yml",
    ),
}
CI_CHANGES_OUTPUTS = {
    "verified": "${{ steps.verified.outputs.verified == 'true' }}",
    "frontend": "${{ steps.verified.outputs.verified != 'true' && steps.filter.outputs.frontend == 'true' }}",
    "rust": "${{ steps.verified.outputs.verified == 'true' && steps.filter.outputs.rust-cache == 'true' || steps.verified.outputs.verified != 'true' && steps.filter.outputs.rust == 'true' }}",
    "github": "${{ steps.verified.outputs.verified != 'true' && steps.filter.outputs.github == 'true' }}",
    # The `rust-macos` matrix: a verified push builds the default suite only.
    "rust_suites": "${{ steps.verified.outputs.verified == 'true' && '[\"default\"]' || '[\"default\",\"internal-benchmark\",\"dictation-shadow\"]' }}",
}
CI_FILTER_RESULT_STEP = """      - name: Require the change filter result
        env:
          FRONTEND: ${{ steps.filter.outputs.frontend }}
          RUST: ${{ steps.filter.outputs.rust }}
          GITHUB_CHANGED: ${{ steps.filter.outputs.github }}
          RUST_CACHE: ${{ steps.filter.outputs.rust-cache }}
        run: |
          for value in "$FRONTEND" "$RUST" "$GITHUB_CHANGED" "$RUST_CACHE"; do
            if [ "$value" != "true" ] && [ "$value" != "false" ]; then
              echo "the change filter produced no answer: '$value'"
              exit 1
            fi
          done
"""
# Each test-running step of `rust-macos` and the matrix suite that runs it. A step must skip
# on a verified push, and must run in exactly one suite: the suites are the feature sets, so a
# step moved to another suite would run its tests with the wrong features.
CI_RUST_TEST_STEPS = {
    "Run tests": "default",
    "Run golden dictation scenarios 100 times": "default",
    "Run admission tests with internal-benchmark": "internal-benchmark",
    "Run dictation shadow tests": "dictation-shadow",
    "Run golden dictation scenarios 100 times with the dictation shadow": "dictation-shadow",
    "Run capture worker unit tests": "default",
    "Run deterministic dictation evaluation": "internal-benchmark",
}
CI_RUST_SUITES = ("default", "internal-benchmark", "dictation-shadow")


def suite_condition(suite: str) -> str:
    return f"needs.changes.outputs.verified != 'true' && matrix.suite == '{suite}'"

# What the dictation-shadow test step must select: the shadow's own tests, its build policy
# tests, E8, the site-order check, and the read and allocation pins.
SHADOW_TEST_SELECTIONS = (
    "dictation_shadow::",
    "dictation_shadow_policy_tests::",
    "every_family_pd_is_witnessed",
    "headers_leave_d",
    "pd_witness_tests::under_d::",
    "the_armed_shadow_adds_nothing_under_d_at_a_process_audio_refusal",
    # The module E8 lives in: its controls (e8_rejects_a_fixture_the_compiled_harness_skips)
    # and any added later run with it, not by name.
    "pd_witness_tests::",
    # The ready-handoff module, with the feature-only skip-reason test. A term that matches
    # nothing is harmless in the union (nextest fails only on an empty selection).
    "ready_handoff_tests::",
    # The lifecycle cutover module (switched entries, the C8.6 re-sync, the not-called checks),
    # which runs in both configs; the shadow step runs it with the comparator armed.
    "lifecycle_cutover_tests::",
    # The stop cutover module (switched entries, the C8.6 re-sync, the not-called checks, the
    # PD-25 winners), which runs in both configs; the shadow step runs it with the comparator armed.
    "stop_cutover_tests::",
    # The cancel cutover module (the switched site-205 entry, the C8.6 re-sync, the not-called
    # checks, R205.5's tail order, the R206 owner and ledger winner for both callers), which runs
    # in both configs; the shadow step runs it with the comparator armed.
    "cancel_cutover_tests::",
    # The start cutover module (the switched site-101/102/103/104 entries and the queries 401-404,
    # the C8.6 re-sync, the not-called checks, the late publish and the scoped cancel), which runs
    # in both configs; the shadow step runs it with the comparator armed.
    "start_cutover_tests::",
)
SEMVER = re.compile(
    r"^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$"
)


def job_block(workflow: str, job: str) -> str:
    match = re.search(
        rf"^  {re.escape(job)}:\n(?P<body>.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)",
        workflow,
        re.MULTILINE | re.DOTALL,
    )
    if not match:
        raise AssertionError(f"missing job: {job}")
    return match.group("body")


def scalar(block: str, key: str) -> str:
    match = re.search(rf"^    {re.escape(key)}:\s*(.+)$", block, re.MULTILINE)
    if not match:
        raise AssertionError(f"missing {key!r} in job block")
    return match.group(1).strip()


def named_step_block(text: str, name: str, indent: int) -> str:
    marker = " " * indent + f"- name: {name}\n"
    start = text.find(marker)
    if start < 0:
        raise AssertionError(f"missing step: {name}")
    next_step = text.find("\n" + " " * indent + "- name:", start + len(marker))
    if next_step < 0:
        next_step = len(text)
    return text[start:next_step]


def step_fields(block: str, indent: int) -> dict[str, str]:
    """The executable fields of one workflow step: `key -> value`, the value of a block scalar
    joined on spaces (a folded `run` is one line to the shell). Comment lines between keys are
    skipped, so a field that was commented out is missing. A step line that is not a field is
    refused, and so is a key given twice."""
    fields: dict[str, list[str]] = {}
    key = None
    for line in block.split("\n")[1:]:
        if not line.strip():
            continue
        pad = len(line) - len(line.lstrip())
        text = line.strip()
        if pad == indent + 2 and not text.startswith("#"):
            match = re.fullmatch(r"([A-Za-z_-]+):\s*(.*)", text)
            if not match or match.group(1) in fields:
                raise AssertionError(f"unreadable or repeated step field: {line!r}")
            key = match.group(1)
            fields[key] = [match.group(2)]
        elif pad > indent + 2 and key is not None:
            fields[key].append(text)
        elif text.startswith("#") and pad >= indent + 2:
            continue
        else:
            raise AssertionError(f"unexpected line in a step: {line!r}")
    values = {}
    for name, parts in fields.items():
        value = " ".join(part for part in parts if part).strip()
        values[name] = re.sub(r"^[>|][-+]?\s*", "", value)
    return values


def validate_exact_step(job: str, name: str, expected: dict[str, str]) -> None:
    """A step that must run exactly as written: its fields are `expected`, no more (no
    `continue-on-error`, `shell`, `working-directory` or any other key), and its `run` text is
    the whole command, so `|| true`, a `;`-chained command, a command in a comment and a
    weakened `if` all fail (the job's failure then reaches `ci-pass`)."""
    fields = step_fields(named_step_block(job, name, 6), 6)
    fields.pop("name", None)
    assert fields == expected, (
        f"step {name!r} must be exactly {expected!r}, found {fields!r}"
    )


def validate_sherpa_refresh(job: str, build_marker: str) -> None:
    """A release-profile Rust cache restore must be followed by the sherpa guard.

    rust-cache can restore a partial hit that keeps the sherpa-onnx-sys build
    fingerprint but not its native libraries, failing the build with "could not
    find native static library". The guard has to run after the restore and
    before the first step that compiles the application.
    """
    guard = named_step_block(job, "Refresh sherpa-onnx native libraries", 6)
    assert "cd app/src-tauri" in guard
    assert "rm -rf target/sherpa-onnx-prebuilt" in guard
    # Release builds use the release profile; plain `cargo clean -p` only
    # cleans the dev profile and would leave the stale fingerprint in place.
    assert "cargo clean --release -p sherpa-onnx-sys" in guard
    restore = job.index("uses: swatinem/rust-cache@v2")
    assert restore < job.index(guard) < job.index(build_marker)


def should_run_ci(event_name: str, head_commit_message: Optional[str]) -> bool:
    return event_name != "push" or not (head_commit_message or "").startswith(
        "chore: bump version"
    )


def should_run_release_build(
    event_name: str, head_commit_message: Optional[str]
) -> bool:
    return event_name == "workflow_dispatch" or (
        event_name == "push"
        and (head_commit_message or "").startswith("chore: bump version")
    )


def should_auto_promote(
    *,
    event_name: str,
    workflow_name: str,
    workflow_path: str,
    conclusion: str,
    head_branch: str,
    source_event: str,
    head_commit_message: Optional[str],
) -> bool:
    return (
        event_name == "workflow_run"
        and workflow_name == "Release Build"
        and workflow_path.split("@", 1)[0] == ".github/workflows/release-build.yml"
        and conclusion == "success"
        and head_branch == "main"
        and source_event == "push"
        and (head_commit_message or "").startswith("chore: bump version")
    )


def release_tag_for_versions(
    tauri_version: str,
    cargo_version: str,
    lock_version: str,
    package_version: str,
    package_lock_version: str,
    changelog_version: str,
) -> str:
    if not SEMVER.fullmatch(tauri_version):
        raise AssertionError(f"invalid release version: {tauri_version}")
    if any(
        version != tauri_version
        for version in (
            cargo_version,
            lock_version,
            package_version,
            package_lock_version,
            changelog_version,
        )
    ):
        raise AssertionError("release version surfaces differ")
    return f"v{tauri_version}"


def tag_action(existing_sha: Optional[str], source_sha: str) -> str:
    if existing_sha is None:
        return "create"
    if existing_sha != source_sha:
        raise AssertionError("existing tag points to a different commit")
    return "reuse"


def validate_ci(ci: str) -> int:
    assert "push:\n    branches: [main]" in ci
    assert "\n  pull_request:" in ci
    changes = job_block(ci, "changes")
    policy = job_block(ci, "policy")
    rust_lint = job_block(ci, "rust-lint")
    rust_macos = job_block(ci, "rust-macos")
    dependency_audit = job_block(ci, "dependency-audit")
    ci_pass = job_block(ci, "ci-pass")
    assert scalar(changes, "if") == CI_GUARD
    # Every build job waits on `changes`; validation belongs in `policy`. The
    # one exception is the push-only verified-tree lookup, a few API calls.
    verified = named_step_block(
        changes, "Check whether this tree already passed on its PR", 6
    )
    assert "if: github.event_name == 'push'" in verified
    assert "run: bash scripts/ci_verified_tree.sh" in verified
    assert changes.count("run:") == 2, "changes must only filter paths"
    assert (
        "verified: ${{ steps.verified.outputs.verified == 'true' }}" in changes
    )
    # A verified push skips tests, never `ci-pass` itself.
    assert (
        "if: needs.changes.outputs.verified != 'true'"
        in named_step_block(rust_lint, "Check Rust formatting", 6)
    ), "rust-lint: Check Rust formatting must skip on a verified push"
    for step, suite in CI_RUST_TEST_STEPS.items():
        assert (
            f"if: {suite_condition(suite)}\n" in named_step_block(rust_macos, step, 6)
        ), f"rust-macos: {step} must skip on a verified push and run only in the {suite} suite"
    # The matrix is exactly the three feature sets, read from the `changes` output (a verified
    # push builds the default suite only), and one suite failing does not hide the others.
    assert re.search(
        r"^    strategy:\n      fail-fast: false\n      matrix:\n"
        r"        suite: \$\{\{ fromJSON\(needs\.changes\.outputs\.rust_suites\) \}\}\n",
        rust_macos,
        re.MULTILINE,
    ), "rust-macos must run one job per feature set from the rust_suites output"
    for suite in CI_RUST_SUITES:
        assert f'"{suite}"' in CI_CHANGES_OUTPUTS["rust_suites"], suite
    assert scalar(policy, "if") == CI_GUARD
    assert "needs:" not in policy, "policy must start without waiting"
    for job in CI_BUILD_JOBS:
        assert scalar(job_block(ci, job), "needs") == "changes"
        assert scalar(job_block(ci, job), "if") == CI_JOB_CONDITIONS[job], (
            f"{job} must run when its change filters select it"
        )
    assert scalar(dependency_audit, "if") == CI_GUARD
    assert scalar(ci_pass, "needs") == f"[changes, policy, {', '.join(CI_BUILD_JOBS)}]"
    assert scalar(ci_pass, "if") == CI_PASS_GUARD
    ci_pass_step = named_step_block(ci_pass, "Check CI result", 6)
    for job in ("changes", "policy", *CI_BUILD_JOBS):
        assert f"${{{{ needs.{job}.result }}}}" in ci_pass_step, job
    # A job its filters select must have succeeded: `ci-pass` accepts a skipped job only when
    # no filter selected it.
    for outputs, jobs, label in CI_SELECTED_JOB_CHECKS:
        condition = " || \\\n             ".join(
            f'[ "${{{{ needs.changes.outputs.{name} }}}}" = "true" ]' for name in outputs
        )
        results = " \\\n              ".join(f'"${{{{ needs.{job}.result }}}}"' for job in jobs)
        selected = (
            f"          if {condition}; then\n"
            f"            for result in \\\n              {results}; do\n"
            '              if [ "$result" != "success" ]; then\n'
            f'                echo "a selected {label} job did not succeed: $result"\n'
            "                exit 1\n"
            "              fi\n"
            "            done\n"
            "          fi\n"
        )
        assert selected in ci_pass_step, f"ci-pass must require success from {jobs} when {outputs} select them"
    # The filter step and what it selects, pinned exactly (a glob that matches nothing, or an
    # output that is always false, would let a Rust-only PR skip both Rust jobs).
    assert "      - uses: dorny/paths-filter@v3\n        id: filter\n        with:\n" in changes
    # The filter step runs on every event: no `if:` (a `github.event_name == 'push'` would skip
    # it on a PR and leave every selector false), and no `continue-on-error`.
    filter_start = changes.index("      - uses: dorny/paths-filter@v3\n")
    filter_end = changes.find("\n      - ", filter_start + 1)
    filter_step = changes[filter_start : filter_end if filter_end >= 0 else len(changes)]
    assert not re.search(r"^        (if|continue-on-error):", filter_step, re.MULTILINE), (
        "the paths-filter step must run unconditionally"
    )
    # And `changes` fails when the filter produced no answer: every output exactly true/false.
    assert CI_FILTER_RESULT_STEP in changes, "changes must require the change filter result"
    for name, expected in CI_FILTER_PATHS.items():
        found = re.search(
            rf"^            {re.escape(name)}:\n(?P<paths>(?:^              (?:- |#).*\n)+)",
            changes,
            re.MULTILINE,
        )
        assert found, f"missing {name} paths filter"
        listed = tuple(
            line.strip()[2:].strip().strip("'")
            for line in found.group("paths").splitlines()
            if line.strip().startswith("- ")
        )
        assert listed == expected, f"the {name} change filter is not the pinned list: {listed}"
    outputs = re.search(r"^    outputs:\n(?P<body>(?:^      .+\n)+)", changes, re.MULTILINE)
    assert outputs, "missing changes outputs"
    declared = dict(
        (line.split(":", 1)[0].strip(), line.split(":", 1)[1].strip())
        for line in outputs.group("body").splitlines()
    )
    assert declared == CI_CHANGES_OUTPUTS, f"the changes outputs are not the pinned expressions: {declared}"
    validation = named_step_block(policy, "Validate workflow policy", 6)
    for marker in (
        "python3 -m pip install -r requirements-test.txt",
        "| xargs -P 6 -I{} python3 scripts/{}.py",
        "            validate_workflow_policy \\\n",
        "            validate_capture_boundary \\\n",
        "            validate_reference_docs \\\n",
        "            validate_markdown_links \\\n",
        "            check_hunt_sites \\\n",
        "            check_shadow_sites \\\n",
        # Every test under tests/ runs, in parallel; a hand-kept list of test files could
        # silently drop a new one.
        "python3 -m pytest tests -q -n auto --dist worksteal\n",
    ):
        assert marker in validation, marker
    assert "python3 -m unittest" not in validation, "the Python tests run through pytest"
    capture_build = named_step_block(
        rust_macos, "Build capture isolation helpers", 6
    )
    assert "uses: dtolnay/rust-toolchain@1.96.0" in named_step_block(
        rust_macos, "Install Rust", 6
    )
    rust_install = named_step_block(rust_lint, "Install Rust", 6)
    assert "uses: dtolnay/rust-toolchain@1.96.0" in rust_install
    assert "components: clippy, rustfmt" in rust_install
    rust_format = named_step_block(rust_lint, "Check Rust formatting", 6)
    assert "cargo fmt --all -- --check" in rust_format
    # Clippy's --all-targets pass is the compile check for tests and examples.
    lint_step = named_step_block(rust_lint, "Lint Rust", 6)
    assert (
        "cargo clippy --workspace --all-targets -- -D warnings"
        in lint_step
    )
    assert "cargo clippy" not in capture_build
    # The shadow feature is off in every other step: its own lint, build and tests are
    # pinned here (D-S2c-15 C1), so a PR cannot drop the only job that compiles it.
    validate_exact_step(
        rust_lint,
        "Lint Rust with the dictation shadow",
        {"run": "cd app/src-tauri && cargo clippy --all-targets --features dictation-shadow -- -D warnings"},
    )
    selection = " | ".join(f"test(/{name}/)" for name in SHADOW_TEST_SELECTIONS)
    # Each feature suite builds its test binary once, with its own feature, and every run step
    # reuses that build through nextest's binaries metadata. A run step without the metadata
    # would build again (about 100 s), and a build without the feature would test the wrong
    # configuration, so both halves are pinned exactly.
    metadata = (
        "cargo nextest run --cargo-metadata target/ci-cargo-metadata.json"
        " --binaries-metadata target/ci-{name}-binaries.json"
    )
    for suite, name, build_step, build_timeout in (
        ("internal-benchmark", "benchmark", "Build internal-benchmark test binaries", "15"),
        ("dictation-shadow", "shadow", "Build dictation shadow test binaries", "25"),
    ):
        validate_exact_step(
            rust_macos,
            build_step,
            {
                "if": suite_condition(suite),
                "timeout-minutes": build_timeout,
                "run": f"cd app/src-tauri && cargo nextest list --lib --features {suite}"
                f" --message-format json --list-type binaries-only > target/ci-{name}-binaries.json"
                " && cargo metadata --format-version 1 > target/ci-cargo-metadata.json",
            },
        )
    validate_exact_step(
        rust_macos,
        "Run admission tests with internal-benchmark",
        {
            "if": suite_condition("internal-benchmark"),
            "timeout-minutes": "15",
            "run": "cd app/src-tauri && " + metadata.format(name="benchmark")
            + " -E 'test(/dictation_admission::/) | test(/corpus::/)'",
        },
    )
    validate_exact_step(
        rust_macos,
        "Run dictation shadow tests",
        {
            "if": suite_condition("dictation-shadow"),
            "timeout-minutes": "25",
            "run": "cd app/src-tauri && " + metadata.format(name="shadow") + f" -E '{selection}'",
        },
    )
    validate_exact_step(
        rust_macos,
        "Run golden dictation scenarios 100 times with the dictation shadow",
        {
            "if": suite_condition("dictation-shadow"),
            "timeout-minutes": "10",
            "env": 'MURMUR_GOLDEN_REPEAT: "100"',
            "run": "cd app/src-tauri && " + metadata.format(name="shadow")
            + " -E 'kind(lib) & test(/golden_tests::golden_/)'",
        },
    )
    for build_step, run_steps in (
        ("Build internal-benchmark test binaries", ("Run admission tests with internal-benchmark",)),
        (
            "Build dictation shadow test binaries",
            (
                "Run dictation shadow tests",
                "Run golden dictation scenarios 100 times with the dictation shadow",
            ),
        ),
    ):
        for run_step in run_steps:
            assert rust_macos.index(f"- name: {build_step}\n") < rust_macos.index(
                f"- name: {run_step}\n"
            ), f"{run_step} must come after the build it reuses"
    # The feature sets stay apart: neither feature is on the default suite's build or runs, and
    # no step turns both on at once.
    default_steps = (
        "Build test binaries",
        "Run tests",
        "Run golden dictation scenarios 100 times",
        "Run capture worker unit tests",
    )
    for step in default_steps:
        block = named_step_block(rust_macos, step, 6)
        assert "--features" not in block, f"{step} is the default suite and takes no feature"
        assert "matrix.suite == 'default'" in block, f"{step} must run only in the default suite"
    assert "--features" not in "".join(
        named_step_block(rust_macos, step, 6)
        for step in ("Build capture isolation helpers",)
    )
    assert "internal-benchmark,dictation-shadow" not in ci and "dictation-shadow,internal-benchmark" not in ci
    # No failure of either job is suppressed, and `ci-pass` waits on both of them.
    for job_name, job in (("rust-lint", rust_lint), ("rust-macos", rust_macos)):
        assert "continue-on-error" not in job, f"{job_name} must not suppress a failure"
        assert f"needs.{job_name}.result" in ci_pass, f"ci-pass must read {job_name}"
    # Only main saves a Rust cache. The suites of `rust-macos` share one cache key, and only the
    # default suite builds the whole dependency graph, so only it saves.
    for job_name, job, save_if in (
        ("rust-lint", rust_lint, "github.ref == 'refs/heads/main'"),
        (
            "rust-macos",
            rust_macos,
            "github.ref == 'refs/heads/main' && matrix.suite == 'default'",
        ),
    ):
        cache = named_step_block(job, "Cache Rust dependencies", 6)
        assert (
            f"save-if: ${{{{ {save_if} }}}}" in cache
        ), f"{job_name} must not save PR caches"
    test_library = named_step_block(rust_macos, "Configure test-only Rust library", 6)
    assert "python3 scripts/configure_ci_rust_library.py" in test_library
    assert rust_macos.index("name: Configure test-only Rust library") < rust_macos.index("name: Cache Rust dependencies")
    assert "configure_ci_rust_library.py" not in rust_lint
    nextest_install = named_step_block(rust_macos, "Install cargo-nextest", 6)
    assert "uses: taiki-e/install-action@v2" in nextest_install
    assert re.search(r"tool: cargo-nextest@[0-9]+\.[0-9]+\.[0-9]+\n", nextest_install)
    # A `--config-file` argument replaces the workspace's `.config/nextest.toml`, and
    # with it the per-test kill budget that keeps a hung test from holding the job.
    ci_commands = "\n".join(
        line for line in ci.splitlines() if not line.lstrip().startswith("#")
    )
    assert (
        "--config-file" not in ci_commands
    ), "a nextest --config-file replaces the per-test timeout in .config/nextest.toml"
    macos_lib_tests = named_step_block(rust_macos, "Run tests", 6)
    # nextest runs each test in its own process, so shared process state stays
    # isolated exactly as `cargo test -- --test-threads=1` guaranteed.
    nextest_reuse = (
        "cargo nextest run --cargo-metadata target/ci-cargo-metadata.json"
        " --binaries-metadata target/ci-test-binaries.json"
    )
    # kind(lib) plus the dictation-core C13.3 bug replays, a separate
    # integration-test target that kind(lib) alone would skip.
    assert (
        nextest_reuse + " -E 'kind(lib) | binary(c13_3_bug_replays)'"
        in " ".join(macos_lib_tests.split())
    )
    # The prebuild must use the run steps' package selection.
    test_build = " ".join(named_step_block(rust_macos, "Build test binaries", 6).split())
    assert (
        "cargo nextest list --workspace --lib"
        " --bin murmur-capture-helper --test topology_watch"
        " --test c13_3_bug_replays"
    ) in test_build
    assert "--example" not in test_build, "the example is built by its own step, in the evaluation suite"
    # The evaluation runs the example binary the step before it built, with the default features.
    validate_exact_step(
        rust_macos,
        "Build the dictation evaluation example",
        {
            "if": suite_condition("internal-benchmark"),
            "run": "cd app/src-tauri && cargo build --workspace --example murmur-eval",
        },
    )
    validate_exact_step(
        rust_macos,
        "Run deterministic dictation evaluation",
        {
            "if": suite_condition("internal-benchmark"),
            "run": "cd app/src-tauri && target/debug/examples/murmur-eval deterministic"
            " --output target/murmur-eval/ci-report.json",
        },
    )
    assert rust_macos.index("- name: Build the dictation evaluation example\n") < rust_macos.index(
        "- name: Run deterministic dictation evaluation\n"
    )
    assert (
        "--message-format json --list-type binaries-only > target/ci-test-binaries.json"
        in test_build
    )
    assert "cargo metadata --format-version 1 > target/ci-cargo-metadata.json" in test_build
    assert "swiftc -warnings-as-errors" in capture_build
    assert "sidecars/capture-agent/main.swift" in capture_build
    assert "cargo build -p murmur-capture-helper" in capture_build
    assert "MURMUR_CAPTURE_ROLE=worker cargo build -p murmur-capture-helper" in capture_build
    assert "MURMUR_CAPTURE_ROLE=helper cargo build -p murmur-capture-helper" in capture_build
    # One shared target directory: a second target dir recompiles the whole
    # dependency graph (webrtc/abseil alone is ~25 s) for a plist-only difference.
    assert "CARGO_TARGET_DIR" not in capture_build
    assert "binaries/murmur-capture-worker-aarch64-apple-darwin" in capture_build
    capture_tests = named_step_block(rust_macos, "Run capture worker unit tests", 6)
    assert (
        nextest_reuse + " -E 'package(=murmur-capture-helper)'"
        in " ".join(capture_tests.split())
    )
    rust_audit = named_step_block(
        dependency_audit, "Audit Rust dependencies (advisory)", 6
    )
    assert "continue-on-error: true" in rust_audit
    assert "cargo audit" in rust_audit
    audit_install = named_step_block(dependency_audit, "Install cargo-audit", 6)
    assert "uses: taiki-e/install-action@v2" in audit_install
    assert "tool: cargo-audit@0.22.2" in audit_install
    npm_audit = named_step_block(
        dependency_audit, "Audit npm dependencies (advisory)", 6
    )
    assert "continue-on-error: true" in npm_audit
    assert "npm audit --audit-level=high" in npm_audit
    cases = (
        ("push", "chore: bump version to 0.17.0", False),
        ("push", "chore: bump version", False),
        ("push", "feat: add a normal feature", True),
        ("pull_request", "chore: bump version to 0.17.0", True),
        ("pull_request", None, True),
    )
    for event_name, message, expected in cases:
        assert should_run_ci(event_name, message) is expected
    return len(cases)


def _kill_budget(slow_timeout: object, where: str) -> int:
    """The seconds a `slow-timeout` setting lets a test run before nextest kills it."""
    assert isinstance(slow_timeout, dict), (
        f"{where}: slow-timeout must be a table with a period and a terminate-after,"
        f" not {slow_timeout!r} (a bare duration only warns and never kills the test)"
    )
    period = slow_timeout.get("period")
    match = re.fullmatch(r"(\d+)s", period) if isinstance(period, str) else None
    assert match and int(match.group(1)) > 0, f"{where}: period must be a positive \"Ns\""
    terminate_after = slow_timeout.get("terminate-after")
    assert (
        isinstance(terminate_after, int)
        and not isinstance(terminate_after, bool)
        and terminate_after > 0
    ), f"{where}: terminate-after must be a positive integer (a timeout without one never kills)"
    return int(match.group(1)) * terminate_after


def validate_nextest_config(config: str) -> None:
    """Every nextest run must kill a test that runs for ever (30 s x 4 = 120 s).

    The default profile needs a kill budget of at most 120 s, and every override (in any
    profile) that sets `slow-timeout` needs a real one too: a warning-only override
    would let a matching test run past the default.
    """
    import tomllib

    parsed = tomllib.loads(config)
    profiles = parsed.get("profile", {})
    default = profiles.get("default")
    assert default is not None, "nextest.toml needs a [profile.default] table"
    assert "slow-timeout" in default, "[profile.default] needs a slow-timeout"
    budget = _kill_budget(default["slow-timeout"], "[profile.default]")
    assert budget <= 120, (
        f"the default per-test kill budget is {budget} s; keep it at 120 s or less and"
        " give a slower test a named override"
    )
    for name, profile in profiles.items():
        for index, override in enumerate(profile.get("overrides", [])):
            if "slow-timeout" in override:
                _kill_budget(
                    override["slow-timeout"],
                    f"[[profile.{name}.overrides]] #{index + 1} ({override.get('filter', '?')})",
                )


def validate_release_build(workflow: str) -> int:
    assert "configure_ci_rust_library.py" not in workflow, "release builds must keep all library outputs"
    assert "push:\n    branches: [main]" in workflow
    assert "\n  workflow_dispatch:" in workflow
    assert "pull_request" not in workflow
    assert "self-hosted" not in workflow
    assert "contents: write" not in workflow
    assert "tests/test_release_version.py" in workflow
    assert (
        "group: release-build-${{ github.event_name }}-${{ github.ref }}-${{ github.sha }}"
        in workflow
    )
    assert scalar(job_block(workflow, "context"), "if") == RELEASE_BUILD_GUARD
    for job in ("typecheck", "release-macos"):
        assert scalar(job_block(workflow, job), "needs") == "context"

    # The native build and frontend verification share only `context`, so both
    # enter the queue concurrently instead of serializing behind typecheck.
    assert "needs: [typecheck]" not in workflow
    assert "macos-release-${{ needs.context.outputs.source-sha }}" in workflow
    assert "capture-helper-tcc-evidence-${{ needs.context.outputs.source-sha }}" in workflow
    assert (
        "--capture-helper-entitlements app/src-tauri/capture-helper.entitlements.plist"
        in workflow
    )
    assert (
        "--capture-agent-entitlements app/src-tauri/capture-agent.entitlements.plist"
        in workflow
    )
    assert "--capture-agent-info-plist app/src-tauri/capture-agent-info.plist" in workflow
    assert (
        "--capture-helper-info-plist app/src-tauri/sidecars/capture/Info.plist"
        in workflow
    )
    assert (
        "--capture-worker-info-plist app/src-tauri/sidecars/capture/WorkerInfo.plist"
        in workflow
    )
    assert (
        "--capture-worker-entitlements app/src-tauri/capture-worker.entitlements.plist"
        in workflow
    )
    worker_smoke = named_step_block(
        workflow, "Smoke test signed capture worker protocol", 6
    )
    assert "scripts/smoke_test_capture_worker.py" in worker_smoke
    assert '--worker "$CAPTURE_WORKER"' in worker_smoke
    assert workflow.index("Finalize, notarize, and repackage") < workflow.index(
        "Smoke test signed capture worker protocol"
    )
    assert workflow.index("Smoke test signed capture worker protocol") < workflow.index(
        "Smoke test signed application"
    )
    assert (
        "--capture-agent-launchd-plist "
        "app/src-tauri/macos/com.localdictation.capture-agent.plist"
        in workflow
    )
    assert '--capture-helper-sha256 "$CAPTURE_HELPER_SHA"' in workflow
    assert '--capture-agent-sha256 "$CAPTURE_AGENT_SHA"' in workflow
    assert '--capture-worker-sha256 "$CAPTURE_WORKER_SHA"' in workflow
    capture_evidence = named_step_block(
        workflow, "Record capture-helper signing and non-interactive probe evidence", 6
    )
    assert "scripts/capture_helper_evidence.py collect-signature" in capture_evidence
    assert "--kind capture-agent" in capture_evidence
    assert "--kind capture-worker" in capture_evidence
    assert "scripts/capture_helper_evidence.py validate-probe" in capture_evidence
    assert '--signed-bundle-artifact "macos-release-$SOURCE_SHA"' in capture_evidence
    assert "signature.json" in capture_evidence
    assert "codesign.txt" not in capture_evidence
    assert "designated-requirement.txt" not in capture_evidence
    assert "shared-key: macos-release-v2" in workflow
    assert "Print :CFBundleExecutable" in workflow
    assert '$(ls "$APP/Contents/MacOS/" | head -1)' not in workflow
    assert workflow.count("${{ needs.context.outputs.cache-write == 'true' }}") == 1
    validate_sherpa_refresh(
        job_block(workflow, "release-macos"), "- name: Build capture helpers"
    )

    cases = (
        ("push", "chore: bump version to 0.17.0", True),
        ("push", "feat: normal merge", False),
        ("pull_request", "chore: bump version to 0.17.0", False),
        ("workflow_dispatch", None, True),
    )
    for event_name, message, expected in cases:
        assert should_run_release_build(event_name, message) is expected
    return len(cases)


def validate_release_rehearsal(workflow: str) -> int:
    assert "name: Release Rehearsal" in workflow
    assert "\n  workflow_dispatch:" in workflow
    for forbidden_trigger in ("\n  push:", "\n  pull_request:", "\n  workflow_run:"):
        assert forbidden_trigger not in workflow
    assert "permissions:\n  contents: read" in workflow
    assert "tests/test_release_version.py" in workflow
    for forbidden in (
        "secrets.",
        "GITHUB_TOKEN",
        "contents: write",
        "actions: write",
        "id-token: write",
        "pull-requests: write",
        "gh release",
        "git tag",
        "tauri-action",
    ):
        assert forbidden not in workflow

    context = job_block(workflow, "context")
    assert "ref: main" in context
    assert 'if [ "$GITHUB_REF" != "refs/heads/main" ]' in workflow
    assert "source_sha must be an exact lowercase 40-character commit SHA" in workflow
    assert 'git fetch --no-tags --depth=1 origin "$SOURCE_SHA"' in workflow
    assert 'RESOLVED=$(git rev-parse "$SOURCE_SHA^{commit}")' in workflow
    assert 'echo "source_sha=$RESOLVED" >> "$GITHUB_OUTPUT"' in workflow
    assert 'echo "workflow_sha=$GITHUB_SHA" >> "$GITHUB_OUTPUT"' in workflow

    for job in ("typecheck", "rehearse-macos"):
        assert scalar(job_block(workflow, job), "needs") == "context"
    assert workflow.count("ref: ${{ needs.context.outputs.source-sha }}") == 2
    assert workflow.count("persist-credentials: false") == 3

    assert (
        "shared-key: macos-release-rehearsal-${{ needs.context.outputs.source-sha }}"
        in workflow
    )
    assert "macos-release-v1" not in workflow
    assert "macos-release-v2" not in workflow
    validate_sherpa_refresh(
        job_block(workflow, "rehearse-macos"), "- name: Build capture helpers"
    )
    assert workflow.count("--no-sign") == 1
    assert '"proxy": "unsigned-release-build"' in workflow
    assert '"workflow_sha": os.environ["WORKFLOW_SHA"]' in workflow
    assert '"source_sha": os.environ["SOURCE_SHA"]' in workflow
    assert '"build_seconds": int(os.environ["BUILD_SECONDS"])' in workflow
    assert workflow.count("uses: actions/upload-artifact@v4") == 1
    return 2


def validate_release_profile(cargo_toml: str) -> None:
    profile = cargo_toml.split("[profile.release]", 1)[1]
    profile = profile.split("\n[", 1)[0]
    assert re.search(r"^strip\s*=\s*false\s*$", profile, re.MULTILINE)


def validate_release_start(workflow: str) -> None:
    assert "name: Release Murmur" in workflow
    assert "\n  workflow_dispatch:" in workflow
    assert "options: [patch, minor]" in workflow
    assert "group: release-start" in workflow
    assert "cancel-in-progress: false" in workflow
    for marker in (
        '[ "$GITHUB_REF" != "refs/heads/main" ]',
        '[ -z "$RELEASE_TOKEN" ]',
        "token: ${{ secrets.RELEASE_TOKEN }}",
        "run: python3 scripts/release_start.py --bump \"$BUMP\" --expected-sha \"$GITHUB_SHA\"",
        "timeout-minutes: 5",
    ):
        assert marker in workflow
    assert "self-hosted" not in workflow


def validate_ota_canary(workflow: str) -> None:
    assert "\n  workflow_call:" in workflow
    assert "\n  workflow_dispatch:" in workflow
    assert "\n  pull_request:" in workflow
    assert scalar(job_block(workflow, "canary"), "runs-on") == "macos-latest"
    assert "contents: read" in workflow
    assert "secrets." not in workflow
    assert "contents: write" not in workflow
    assert "self-hosted" not in workflow
    assert "--remote --isolated" in workflow
    assert '--root "$RUNNER_TEMP/murmur-ota-canary"' in workflow
    assert "ref: ${{ inputs.source_sha || github.sha }}" in workflow
    assert "persist-credentials: false" in workflow
    assert "--dry-run" not in workflow
    assert "timeout-minutes: 20" in workflow


def validate_promotion_policy(workflow: str) -> int:
    assert "tags:\n      - 'v*'" in workflow
    assert "workflow_run:\n    workflows: [Release Build]\n    types: [completed]" in workflow
    assert "\n  workflow_dispatch:" in workflow
    assert "self-hosted" not in workflow
    canary = job_block(workflow, "ota-canary")
    assert "needs: [resolve, promote]" in canary
    assert "needs.resolve.outputs.publish == 'true'" in canary
    assert "needs.promote.result == 'success'" in canary
    assert "needs.resolve.outputs.already-published == 'true'" in canary
    assert "uses: ./.github/workflows/ota-canary.yml" in canary
    assert "source_sha: ${{ needs.resolve.outputs.source-sha }}" in canary
    assert "tag: ${{ needs.resolve.outputs.tag }}" in canary
    assert "actions/cache" not in workflow
    assert "swatinem/rust-cache" not in workflow
    assert scalar(job_block(workflow, "promote"), "needs") == "resolve"
    assert scalar(job_block(workflow, "promote"), "if") == (
        "needs.resolve.outputs.eligible == 'true' && "
        "needs.resolve.outputs.already-published != 'true'"
    )
    assert "github.event.workflow_run.head_sha || github.sha" in workflow
    for gate in (
        'WORKFLOW_RUN_NAME: ${{ github.event.workflow_run.name }}',
        'WORKFLOW_RUN_PATH: ${{ github.event.workflow_run.path }}',
        'WORKFLOW_RUN_BRANCH: ${{ github.event.workflow_run.head_branch }}',
        'WORKFLOW_RUN_EVENT: ${{ github.event.workflow_run.event }}',
        'WORKFLOW_RUN_CONCLUSION: ${{ github.event.workflow_run.conclusion }}',
        '[ "$WORKFLOW_RUN_NAME" != "Release Build" ]',
        '[ "$WORKFLOW_RUN_CONCLUSION" != "success" ]',
        '[ "$WORKFLOW_RUN_BRANCH" != "main" ]',
        '[ "$WORKFLOW_RUN_EVENT" != "push" ]',
        '[[ "$SUBJECT" != "chore: bump version"* ]]',
    ):
        assert gate in workflow
    assert "head_branch == \"main\"" in workflow
    assert ".head_sha == $sha" in workflow
    assert ".event == \"push\"" in workflow
    assert 'split("@")[0]) == ".github/workflows/release-build.yml"' in workflow
    assert "expired == false" in workflow
    assert "scripts/release_artifacts.py validate" in workflow
    assert 'for NAME in "macos-release-${SOURCE_SHA}"; do' in workflow
    assert "scripts/release_version.py check" in workflow
    assert '--git-ref "$SOURCE_SHA"' in workflow
    assert "--require-macos-capture-helper" in workflow
    assert "--require-macos-capture-agent" in workflow
    assert "--require-macos-capture-worker" in workflow
    assert "release versions or CHANGELOG differ" in workflow
    assert "already_published=true" in workflow
    assert "contains unexpected asset" in workflow
    assert 'gh release view "$TAG"' in workflow
    assert "--json body --jq .body > release-notes.md" in workflow
    assert "--release-notes release-notes.md" in workflow
    assert ".github/updater-policy.json" in workflow
    assert "updater policy must contain exactly one null or string min_version" in workflow
    assert '--min-version "$MIN_VERSION"' in workflow
    assert "published updater policy differs from the trusted source policy" in workflow
    assert workflow.index("scripts/release_artifacts.py validate") < workflow.index(
        "Create automatic release tag"
    )

    publish_steps = (
        "Create automatic release tag",
        "Create or reuse draft release",
        "Upload signed release assets",
        "Verify uploaded updater signatures",
        "Generate updater channel manifests from verified signatures",
        "Upload and verify updater manifests",
        "Verify release metadata matches updater manifest",
        "Publish release",
        "Promote highest published release to latest",
    )
    for name in publish_steps:
        block = named_step_block(workflow, name, 6)
        assert "if: needs.resolve.outputs.publish == 'true'" in block
    assert workflow.index("Verify release metadata matches updater manifest") < workflow.index(
        "Publish release"
    )
    # Concurrent version builds can finish out of order, so publication never
    # lets GitHub pick "latest"; the promotion step converges on the highest.
    publish = named_step_block(workflow, "Publish release", 6)
    assert "--draft=false --latest=false" in publish
    promote = named_step_block(workflow, "Promote highest published release to latest", 6)
    assert "scripts/release_artifacts.py highest-release" in promote
    assert "--exclude-drafts --exclude-pre-releases" in promote
    assert workflow.index("Publish release") < workflow.index(
        "Promote highest published release to latest"
    )
    metadata_check = named_step_block(
        workflow, "Verify release metadata matches updater manifest", 6
    )
    assert "scripts/release_artifacts.py verify-notes" in metadata_check
    assert "--json body --jq .body > draft-release-notes.md" in metadata_check
    assert "--manifest remote-manifests/latest-v2.json" in metadata_check
    assert "--release-notes draft-release-notes.md" in metadata_check
    assert "RELEASE_NOTES=$(" not in metadata_check
    assert ".notes == $notes" not in metadata_check
    rehearsal = named_step_block(
        workflow, "Report non-publishing promotion rehearsal", 6
    )
    assert "if: needs.resolve.outputs.publish != 'true'" in rehearsal
    trusted = dict(
        event_name="workflow_run",
        workflow_name="Release Build",
        workflow_path=".github/workflows/release-build.yml",
        conclusion="success",
        head_branch="main",
        source_event="push",
        head_commit_message="chore: bump version to 0.18.0",
    )
    auto_cases = (
        trusted,
        {**trusted, "source_event": "workflow_dispatch"},
        {**trusted, "head_branch": "feature"},
        {**trusted, "conclusion": "failure"},
        {**trusted, "head_commit_message": "fix: ordinary main commit"},
    )
    expected = (True, False, False, False, False)
    for case, result in zip(auto_cases, expected):
        assert should_auto_promote(**case) is result
    assert release_tag_for_versions(*(["0.18.0"] * 6)) == "v0.18.0"
    assert tag_action(None, "a" * 40) == "create"
    assert tag_action("a" * 40, "a" * 40) == "reuse"
    return len(publish_steps) + len(auto_cases)


DEV_SIGNING_MARKERS = ("MURMUR_DEV_SIGNING_IDENTITY", "build_dev_bundle")


def validate_dev_signing_isolation(workflows: dict[str, str]) -> int:
    """Keep the opt-in dev signing identity out of CI and release paths."""
    for name, workflow in workflows.items():
        for marker in DEV_SIGNING_MARKERS:
            assert marker not in workflow, f"{name} must not use dev signing ({marker})"
    return len(workflows)


def main() -> None:
    ci = CI_WORKFLOW.read_text()
    release_build = RELEASE_BUILD_WORKFLOW.read_text()
    release_rehearsal = RELEASE_REHEARSAL_WORKFLOW.read_text()
    release = RELEASE_WORKFLOW.read_text()
    cargo_toml = CARGO_TOML.read_text()

    ci_cases = validate_ci(ci)
    validate_nextest_config(NEXTEST_CONFIG.read_text())
    release_build_cases = validate_release_build(release_build)
    rehearsal_jobs = validate_release_rehearsal(release_rehearsal)
    validate_release_profile(cargo_toml)
    validate_release_start(RELEASE_START_WORKFLOW.read_text())
    validate_ota_canary(OTA_CANARY_WORKFLOW.read_text())
    publication_steps = validate_promotion_policy(release)
    validate_dev_signing_isolation(
        {
            path.name: path.read_text()
            for path in sorted((ROOT / ".github/workflows").glob("*.yml"))
        }
    )

    print(
        "workflow policy validation passed "
        f"({ci_cases} CI cases; {release_build_cases} release-build cases; "
        f"{rehearsal_jobs} secretless rehearsal jobs; "
        f"{publication_steps} publication gates; trusted cache ownership intact)"
    )


if __name__ == "__main__":
    main()
