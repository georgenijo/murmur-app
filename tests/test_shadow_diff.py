from pathlib import Path
import contextlib
import io
import subprocess
import tempfile
import unittest

from scripts.check_shadow_diff import (
    SHADOW_DIFF_ALLOW,
    _cfg_value,
    SHADOW_ONLY,
    guarded_paths,
    is_production_path,
    is_shadow_only,
    is_test_file,
    main,
    numstat,
    violations,
)

LIVE = "app/src-tauri/src/keyboard.rs"
FIXTURES = Path(__file__).resolve().parent / "fixtures" / "shadow_twins"


@contextlib.contextmanager
def allowed(twins: dict):
    """Run with `SHADOW_TWINS` replaced by `twins` (the real table names the real tree)."""
    from unittest import mock

    import scripts.check_shadow_diff as module

    with mock.patch.dict(module.SHADOW_TWINS, twins, clear=True):
        yield


def git(root: Path, *args: str) -> None:
    subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@example.com", *args],
        cwd=root,
        check=True,
        capture_output=True,
    )


_KEEP: list[tempfile.TemporaryDirectory] = []


def repo(base_text: str, branch_text: str) -> Path:
    tmp = tempfile.TemporaryDirectory()
    _KEEP.append(tmp)
    root = Path(tmp.name)
    git(root, "init", "-b", "main")
    (root / LIVE).parent.mkdir(parents=True)
    (root / LIVE).write_text(base_text)
    git(root, "add", ".")
    git(root, "commit", "-m", "base")
    git(root, "checkout", "-b", "branch")
    (root / LIVE).write_text(branch_text)
    git(root, "commit", "-am", "branch")
    return root


def run(root: Path) -> tuple[int, str, str]:
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = main(root, "main")
    return code, out.getvalue(), err.getvalue()


class ShadowDiffTests(unittest.TestCase):
    def test_an_insertion_passes(self):
        root = repo("a\nb\n", "a\n#[cfg(feature = \"dictation-shadow\")]\nx();\nb\n")
        code, out, err = run(root)
        self.assertEqual(code, 0, err)
        self.assertIn(f"2\t0\t{LIVE}", out)

    def test_a_removed_line_fails(self):
        root = repo("a\nb\nc\n", "a\nc\n")
        code, _, err = run(root)
        self.assertEqual(code, 1)
        self.assertIn(LIVE, err)
        self.assertIn("removes 1 lines", err)

    def test_a_modified_line_fails(self):
        root = repo("a\nb\n", "a\nb2\n")
        code, _, err = run(root)
        self.assertEqual(code, 1)
        self.assertIn("removes 1 lines", err)

    def test_a_moved_statement_fails(self):
        root = repo("a\nb\nc\n", "b\na\nc\n")
        code, _, err = run(root)
        self.assertEqual(code, 1, err)

    def test_a_file_outside_the_live_path_may_change(self):
        stats = {"app/src-tauri/src/dictation_shadow/mod.rs": (10, 9)}
        self.assertEqual(violations(stats), [])

    def test_the_allow_list_names_a_file_a_count_and_a_reason(self):
        self.assertEqual(
            list(SHADOW_DIFF_ALLOW),
            ["app/src-tauri/src/main_thread_watch.rs"],
            "an allowed removal is a reviewed exception, named one by one",
        )
        for count, reason in SHADOW_DIFF_ALLOW.values():
            self.assertEqual(count, 1)
            self.assertIn("test-only", reason)
        self.assertEqual(violations({}), [])
        stats = {LIVE: (3, 2)}
        self.assertTrue(violations(stats))
        self.assertEqual(violations(stats, allow={LIVE: (2, "a renamed import")}), [])
        self.assertTrue(violations(stats, allow={LIVE: (1, "too few")}))
        self.assertTrue(violations({}, allow={LIVE: (1, " ")}))
        self.assertTrue(violations({}, allow={"app/other.rs": (1, "not a production file")}))

    def test_every_production_path_is_guarded_without_a_list_to_forget(self):
        for path in [
            "app/src-tauri/src/commands/recording.rs",
            "app/src-tauri/src/commands/recording_host.rs",
            "app/src-tauri/src/anything/new.rs",
            "app/src-tauri/crates/dictation-core/src/decision.rs",
            "app/src-tauri/crates/another/src/deep/file.rs",
        ]:
            self.assertTrue(is_production_path(path), path)
            self.assertTrue(violations({path: (0, 1)}), path)
        for path in ["scripts/x.py", "app/src/App.tsx", "app/src-tauri/build.rs"]:
            self.assertFalse(is_production_path(path), path)

    def test_test_files_and_shadow_only_modules_are_not_guarded(self):
        self.assertTrue(is_test_file("app/src-tauri/src/a_tests.rs", ""))
        self.assertTrue(is_test_file("app/src-tauri/src/x/tests.rs", ""))
        self.assertTrue(is_test_file("app/src-tauri/src/x/tests/y.rs", ""))
        self.assertTrue(is_test_file("app/src-tauri/src/z.rs", "//! doc\n#![cfg(test)]\n"))
        self.assertFalse(is_test_file("app/src-tauri/src/z.rs", "fn f() {}\n"))
        # A `#[cfg(test)]` on an inner item does not make a production file a test file.
        self.assertFalse(is_test_file("app/src-tauri/src/z.rs", "#[cfg(test)]\nmod t {}\n"))
        for entry in SHADOW_ONLY:
            self.assertTrue(is_shadow_only(entry if not entry.endswith("/") else entry + "x.rs"))
        self.assertFalse(is_shadow_only("app/src-tauri/src/commands/recording.rs"))

    def test_a_deletion_in_a_file_the_old_list_did_not_name_fails(self):
        path = "app/src-tauri/src/commands/recording_host.rs"
        root = repo("a\nb\nc\n", "a\nb\nc\nd\n")
        (root / path).parent.mkdir(parents=True, exist_ok=True)
        (root / path).write_text("fn a() {}\nfn b() {}\nfn c() {}\n")
        git(root, "add", ".")
        git(root, "commit", "-m", "host on main")
        git(root, "checkout", "-q", "main")
        git(root, "merge", "-q", "--ff-only", "branch")
        git(root, "checkout", "-q", "-b", "simulated")
        (root / path).write_text("fn a() {}\nfn c() {}\n")
        git(root, "commit", "-am", "delete a line of the host")
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = main(root, "main")
        self.assertEqual(code, 1, out.getvalue())
        self.assertIn(path, err.getvalue())
        self.assertIn("removes 1 lines", err.getvalue())

    def test_a_renamed_file_is_a_deletion_plus_an_addition(self):
        old = "app/src-tauri/src/commands/recording_host.rs"
        new = "app/src-tauri/src/commands/recording_host_moved.rs"
        root = repo("a\n", "a\nb\n")
        (root / old).parent.mkdir(parents=True, exist_ok=True)
        (root / old).write_text("fn a() {}\nfn b() {}\n")
        git(root, "add", ".")
        git(root, "commit", "-m", "host on main")
        git(root, "checkout", "-q", "main")
        git(root, "merge", "-q", "--ff-only", "branch")
        git(root, "checkout", "-q", "-b", "renamed")
        git(root, "mv", old, new)
        git(root, "commit", "-m", "rename the host")
        stats = numstat(root, "main")
        self.assertEqual(stats[old], (0, 2), "the old path is a deletion")
        self.assertEqual(stats[new], (2, 0), "the new path is an addition")
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = main(root, "main")
        self.assertEqual(code, 1, out.getvalue())
        self.assertIn(old, err.getvalue())

    def test_a_test_file_and_a_shadow_module_may_change_freely(self):
        root = repo("a\n", "a\nb\n")
        files = {
            "app/src-tauri/src/commands/recording_foo_tests.rs": "x\ny\n",
            "app/src-tauri/src/dictation_shadow/mod.rs": "p\nq\n",
            "app/src-tauri/src/harnessy.rs": "#![cfg(test)]\nr\ns\n",
        }
        for path, text in files.items():
            (root / path).parent.mkdir(parents=True, exist_ok=True)
            (root / path).write_text(text)
        git(root, "add", ".")
        git(root, "commit", "-m", "base files")
        git(root, "checkout", "-q", "main")
        git(root, "merge", "-q", "--ff-only", "branch")
        git(root, "checkout", "-q", "-b", "edits")
        for path in files:
            keeps = "#![cfg(test)]\n" if "harnessy" in path else ""
            (root / path).write_text(keeps + "changed\n")
        git(root, "commit", "-am", "edit test and shadow code")
        stats = numstat(root, "main")
        self.assertEqual(guarded_paths(stats, root, "main"), [])
        code, _, err = run(root)
        self.assertEqual(code, 0, err)

    def test_always_false_gates_are_recognised(self):
        for predicate in [
            "any()",
            "not(all())",
            "all(any())",
            "all(feature = \"a\", any())",
            "any(any(), any())",
            "not(not(any()))",
        ]:
            self.assertIs(_cfg_value(predicate), False, predicate)
        for predicate in [
            "test",
            "feature = \"dictation-shadow\"",
            "not(feature = \"dictation-shadow\")",
            "any(test, any())",
            "all()",
        ]:
            self.assertIsNot(_cfg_value(predicate), False, predicate)

    def test_an_always_false_gate_on_an_existing_statement_fails(self):
        for gate in ["#[cfg(any())]", "#[cfg(not(all()))]", "#[cfg(all(any()))]"]:
            root = repo("a\nb\nc\n", f"a\n{gate}\nb\nc\n")
            code, _, err = run(root)
            self.assertEqual(code, 1, f"{gate}: {err}")
            self.assertIn("is not the one permitted gate", err)

    def test_an_always_false_gate_on_a_new_statement_is_harmless(self):
        root = repo("a\nb\n", "a\n#[cfg(any())]\nnew();\nb\n")
        code, _, err = run(root)
        self.assertEqual(code, 0, err)

    def test_every_inserted_cfg_on_an_existing_statement_is_rejected_unevaluated(self):
        # D-S2c-12 c: nothing but the negated twin gate is accepted on an existing statement,
        # whatever it evaluates to, so a gate that merely holds is rejected as well.
        for gate in [
            '#[cfg(feature = "dictation-shadow")]',
            "#[cfg(not(test))]",
            "#[cfg(test)]",
            "#[cfg(all(test, unix))]",
            '#[cfg(any(not(feature = "dictation-shadow"), unix))]',
            '#[cfg_attr(feature = "dictation-shadow", inline)]',
            '#[cfg_attr(test, allow(dead_code))]',
        ]:
            root = repo("a\nb\nc\n", f"a\n{gate}\nb\nc\n")
            code, _, err = run(root)
            self.assertEqual(code, 1, f"{gate}: {err}")
            self.assertIn("is not the one permitted gate", err, gate)

    def test_sols_four_spellings_of_the_negated_gate_fail(self):
        feature = 'feature = "dictation-shadow"'
        for gate in [
            f"#[cfg(not ({feature}))]",
            '#[cfg(not(feature = r"dictation-shadow"))]',
            f'#[cfg(any(not({feature}), target_os = "linux"))]',
            f"#[cfg_attr({feature}, cfg(any()))]",
        ]:
            root = repo("a\nb\nc\n", f"a\n{gate}\nb\nc\n")
            code, _, err = run(root)
            self.assertEqual(code, 1, f"{gate}: {err}")
        # Even on an allowlisted twin, only the exact attribute is accepted: the raw string,
        # the `any(..)` and the `cfg_attr` spellings are not it.
        twin = '#[cfg(feature = "dictation-shadow")]'
        for gate in [
            '#[cfg(not(feature = r"dictation-shadow"))]',
            f'#[cfg(any(not({feature}), target_os = "linux"))]',
            f"#[cfg_attr({feature}, cfg(any()))]",
        ]:
            with allowed({LIVE: [{"first": "b();", "form": "closed", "why": "twin"}]}):
                code, _, err = run(repo("a();\nb();\n", f"a();\n{gate}\nb();\n{twin}\nb();\n"))
            self.assertEqual(code, 1, f"{gate}: {err}")
            self.assertIn("is not the one permitted gate", err, gate)

    def test_a_negated_shadow_gate_on_an_unlisted_statement_fails(self):
        feature = 'feature = "dictation-shadow"'
        for gate in [
            f"#[cfg(not({feature}))]",
            f"#[cfg(not({feature}))] // keep",
            f"#[cfg(\n    not({feature})\n)]",
            f"#[cfg(not ({feature}))]",
        ]:
            root = repo("a\nb\nc\n", f"a\n{gate}\nb\nc\n")
            code, _, err = run(root)
            self.assertEqual(code, 1, f"{gate!r}: {err}")
            self.assertIn("is not in SHADOW_TWINS", err)
        for gate in [f"#[cfg(all(not({feature}), unix))]"]:
            code, _, err = run(repo("a\nb\nc\n", f"a\n{gate}\nb\nc\n"))
            self.assertEqual(code, 1, f"{gate!r}: {err}")

    def test_the_auto_stop_pattern_of_dca44211_fails(self):
        # The original await and its diagnostic switched off in the feature build, a twin
        # beside it that runs the witnessed result instead.
        base = (
            "async fn run_auto_stop(recording_id: u64, stop: AutoStopTask) {\n"
            "    if let Some(Err(_)) = stop.await {\n"
            '        tracing::error!(recording_id, "failed");\n'
            "    }\n"
            "}\n"
        )
        branch = (
            "async fn run_auto_stop(recording_id: u64, stop: AutoStopTask) {\n"
            '    #[cfg(not(feature = "dictation-shadow"))]\n'
            "    if let Some(Err(_)) = stop.await {\n"
            '        tracing::error!(recording_id, "failed");\n'
            "    }\n"
            '    #[cfg(feature = "dictation-shadow")]\n'
            "    if let Some(Err(_)) = witnessed_auto_stop(stop).await {\n"
            '        tracing::error!(recording_id, "failed");\n'
            "    };\n"
            "}\n"
        )
        code, _, err = run(repo(base, branch))
        self.assertEqual(code, 1, err)
        self.assertIn("is not in SHADOW_TWINS", err)
        # The accepted shape: the original kept in both builds, one gated rebinding before it.
        branch = (
            "async fn run_auto_stop(recording_id: u64, stop: AutoStopTask) {\n"
            '    #[cfg(feature = "dictation-shadow")]\n'
            "    let stop = witness_detached(stop);\n"
            "    if let Some(Err(_)) = stop.await {\n"
            '        tracing::error!(recording_id, "failed");\n'
            "    }\n"
            "}\n"
        )
        code, _, err = run(repo(base, branch))
        self.assertEqual(code, 0, err)

    def test_a_handler_entry_passes_and_only_a_named_statement_may_be_gated_off(self):
        gate = '#[cfg(not(feature = "dictation-shadow"))]'
        twin = '#[cfg(feature = "dictation-shadow")]'
        branch = f"a\n{gate}\nhandler,\n{twin}\nwrapper,\nb\n"
        base = "a\nhandler,\nb\n"
        entry = {"first": "handler,", "form": "handler", "why": "wrapper of the same name"}
        with allowed({LIVE: [entry]}):
            code, _, err = run(repo(base, branch))
            self.assertEqual(code, 0, err)
            # A second statement of the same text, or any other statement, is not named.
            two = f"{gate}\nhandler,\n{twin}\nwrapper,\n"
            code, _, err = run(repo("handler,\nhandler,\n", two + two))
            self.assertEqual(code, 1, err)
            code, _, err = run(repo("a\nb\n", f"{gate}\na\n{twin}\nx,\nb\n"))
            self.assertEqual(code, 1, err)

    def test_the_twin_table_names_a_production_file_a_form_and_a_reason(self):
        from scripts.check_shadow_diff import SHADOW_TWINS, twin_table_problems

        self.assertEqual(twin_table_problems(), [])
        for path in SHADOW_TWINS:
            self.assertTrue(is_production_path(path), path)
        # D-S2c-8 amendment 1: the generate_handler! entries are the named exceptions.
        handlers = [e for e in SHADOW_TWINS["app/src-tauri/src/lib.rs"] if e["form"] == "handler"]
        self.assertEqual(len(handlers), 4)
        for entry in SHADOW_TWINS["app/src-tauri/src/commands/recording.rs"]:
            self.assertIn(entry["form"], ("closed", "frozen"))
        self.assertTrue(twin_table_problems({"app/other.rs": []}))
        self.assertTrue(twin_table_problems({LIVE: [{"first": "a", "form": "closed", "why": " "}]}))
        self.assertTrue(twin_table_problems({LIVE: [{"first": "a", "form": "loose", "why": "x"}]}))
        self.assertTrue(twin_table_problems({LIVE: [{"first": "a", "form": "frozen", "why": "x"}]}))

    # ------------------------------------------------- twin equivalence (D-S2c-12 b)

    def closed(self, base: str, twin: str, first: str):
        gate = '#[cfg(not(feature = "dictation-shadow"))]'
        feature = '#[cfg(feature = "dictation-shadow")]'
        branch = f"fn f() {{\n{gate}\n{base}{feature}\n{twin}}}\n"
        entry = {"first": first, "form": "closed", "why": "twin"}
        with allowed({LIVE: [entry]}):
            return run(repo(f"fn f() {{\n{base}}}\n", branch))

    def test_a_twin_that_is_the_original_or_one_permitted_insertion_passes(self):
        original = "run_phase(&mut executor(&ports, tail), Placement::PostDT, None, rid);\n"
        for twin in [
            original,
            "run_phase(&mut executor(&ports, tail).attach_flight(&flight), Placement::PostDT, None, rid);\n",
            "run_phase(&mut executor(&ports, tail), Placement::PostDT, None, rid, flight);\n",
        ]:
            code, _, err = self.closed(original, twin, original.strip())
            self.assertEqual(code, 0, f"{twin!r}: {err}")

    def test_a_shadow_statement_beside_a_twin_is_a_plain_insertion(self):
        original = "run_phase(&mut executor(&ports, tail), Placement::PostDT, None, rid);\n"
        feature = '#[cfg(feature = "dictation-shadow")]'
        branch = f"fn f() {{\n{feature}\nshadow::note(&flight);\n{original}}}\n"
        code, _, err = run(repo(f"fn f() {{\n{original}}}\n", branch))
        self.assertEqual(code, 0, err)

    def test_a_twin_that_differs_by_anything_else_fails(self):
        original = "run_phase(&mut executor(&ports, tail), Placement::PostDT, None, rid);\n"
        for twin in [
            # argument order
            "run_phase(&mut executor(&ports, tail), Placement::PostDT, rid, None);\n",
            # `?`
            "run_phase(&mut executor(&ports, tail), Placement::PostDT, None, rid)?;\n",
            # a block boundary
            "{ run_phase(&mut executor(&ports, tail), Placement::PostDT, None, rid); }\n",
            # a `let` ordering
            "let at = core_now();\n" + original,
            # a clock read moved into the call
            "run_phase(&mut executor(&ports, tail), Placement::PostDT, None, rid, core_now());\n",
            # two insertions
            "run_phase(&mut executor(&ports, tail).attach_flight(&flight), Placement::PostDT, None, rid, f2);\n",
            # an inserted call that is not a `shadow::` statement
            "other::note(&flight);\n" + original,
            # an inserted attach that does not end an expression
            "run_phase(&mut executor(&ports, tail), Placement::PostDT, None.attach_flight(&flight), rid);\n",
            # an extra argument that is a call
            "run_phase(&mut executor(&ports, tail), Placement::PostDT, None, rid, flight());\n",
            # a shorter twin
            "run_phase(&mut executor(&ports, tail), Placement::PostDT, None);\n",
        ]:
            code, _, err = self.closed(original, twin, original.strip())
            self.assertEqual(code, 1, f"{twin!r}: {err}")
            self.assertIn("differs from the original", err, twin)

    def test_another_attribute_stacked_on_a_twin_fails(self):
        # Round 9 nit 3: `#[cfg(any())]` right after the feature gate switches the twin off
        # with zero deletions; the attribute stack is part of what is compared.
        original = "run_phase(&mut executor(&ports, tail), Placement::PostDT, None, rid);\n"
        attached = "run_phase(&mut executor(&ports, tail).attach_flight(&flight), Placement::PostDT, None, rid);\n"
        for stack in ["#[cfg(any())]\n", "#[allow(unused)]\n", "#[cfg(test)]\n#[cfg(any())]\n"]:
            code, _, err = self.closed(original, stack + attached, original.strip())
            self.assertEqual(code, 1, f"{stack!r}: {err}")
            self.assertIn("differs from the original", err, stack)
        code, _, err = self.closed(original, attached, original.strip())
        self.assertEqual(code, 0, err)
        # A handler entry may not carry one either.
        gate = '#[cfg(not(feature = "dictation-shadow"))]'
        feature = '#[cfg(feature = "dictation-shadow")]'
        entry = {"first": "handler,", "form": "handler", "why": "wrapper"}
        with allowed({LIVE: [entry]}):
            code, _, err = run(
                repo("a\nhandler,\nb\n", f"a\n{gate}\nhandler,\n{feature}\n#[cfg(any())]\nwrapper,\nb\n")
            )
        self.assertEqual(code, 1, err)
        self.assertIn("carries another attribute", err)

    def test_a_gated_block_around_an_original_statement_fails(self):
        # Round 9 nit 4: a new gated block that encloses an existing statement removes it
        # from the default build with zero deleted lines.
        base = "fn f() {\n    let before = a();\n    counter.fetch_max(id, Ordering::SeqCst);\n    after();\n}\n"
        wrapped = (
            "fn f() {\n    let before = a();\n"
            '    #[cfg(feature = "dictation-shadow")]\n    {\n'
            "    counter.fetch_max(id, Ordering::SeqCst);\n"
            "    }\n    after();\n}\n"
        )
        code, _, err = run(repo(base, wrapped))
        self.assertEqual(code, 1, err)
        self.assertIn("a gated region encloses text the base already had", err)
        # Any gate shape, and a gated `if` around it, are the same.
        for gate in ["#[cfg(test)]", "#[cfg(any())]", "#[cfg(not(test))]"]:
            branch = wrapped.replace('#[cfg(feature = "dictation-shadow")]', gate)
            code, _, err = run(repo(base, branch))
            self.assertEqual(code, 1, f"{gate}: {err}")
        # A new gated statement beside the original, even one that repeats its text, is fine.
        beside = (
            "fn f() {\n    let before = a();\n"
            '    #[cfg(feature = "dictation-shadow")]\n'
            "    counter.fetch_max(id, Ordering::SeqCst);\n"
            "    counter.fetch_max(id, Ordering::SeqCst);\n    after();\n}\n"
        )
        code, _, err = run(repo(base, beside))
        self.assertEqual(code, 0, err)
        # A gated list element and a gated match arm take only themselves out.
        code, _, err = run(
            repo(
                "fn f() {\n    list![\n        a,\n        b,\n    ];\n}\n",
                'fn f() {\n    list![\n        a,\n        #[cfg(feature = "dictation-shadow")]\n        w,\n        b,\n    ];\n}\n',
            )
        )
        self.assertEqual(code, 0, err)
        code, _, err = run(
            repo(
                "fn f() {\n    match x {\n        A => 1,\n        B => { 2 }\n    }\n}\n",
                'fn f() {\n    match x {\n        A => 1,\n        #[cfg(feature = "dictation-shadow")]\n        W => { 9 }\n        B => { 2 }\n    }\n}\n',
            )
        )
        self.assertEqual(code, 0, err)

    def test_a_missing_twin_fails(self):
        gate = '#[cfg(not(feature = "dictation-shadow"))]'
        entry = {"first": "b();", "form": "closed", "why": "twin"}
        with allowed({LIVE: [entry]}):
            code, _, err = run(repo("a();\nb();\n", f"a();\n{gate}\nb();\n"))
        self.assertEqual(code, 1, err)
        self.assertIn("twin of the statement", err)

    def test_the_seven_round_8_twins_fail(self):
        # The shapes Sol found at a4bc9d8d (D-S2c-12): each differs from its original by a
        # clock placement and an ungated cell, a reply rendered while the executor lives, or
        # a leaf guard held over a witness's construction.
        names = [
            "guard_finish_clock",
            "pipeline_idle_clock",
            "stop_capture_clock",
            "start_refusal_pre",
            "start_refusal_decision",
            "clear_context_scope",
            "set_context_scope",
        ]
        for name in names:
            original = (FIXTURES / f"{name}.orig.rs").read_text()
            twin = (FIXTURES / f"{name}.twin.rs").read_text()
            first = original.splitlines()[0].strip()
            code, _, err = self.closed(original, twin, first)
            self.assertEqual(code, 1, f"{name}: {err}")
            self.assertIn("differs from the original", err, name)

    def test_the_seven_round_8_twins_fail_even_when_frozen_to_a_stale_digest(self):
        for name in ["guard_finish_clock", "set_context_scope"]:
            original = (FIXTURES / f"{name}.orig.rs").read_text()
            twin = (FIXTURES / f"{name}.twin.rs").read_text()
            gate = '#[cfg(not(feature = "dictation-shadow"))]'
            branch = f"fn f() {{\n{gate}\n{original}{twin}}}\n"
            entry = {
                "first": original.splitlines()[0].strip(),
                "form": "frozen",
                "pair": "0" * 16,
                "why": "x",
            }
            with allowed({LIVE: [entry]}):
                code, _, err = run(repo(f"fn f() {{\n{original}}}\n", branch))
            self.assertEqual(code, 1, f"{name}: {err}")
            self.assertIn("is not the pair a reviewer accepted", err)

    def test_a_frozen_pair_passes_only_with_its_own_digest(self):
        from scripts.check_shadow_diff import normalised, pair_hash, tokenize

        original = "x();\n"
        twin = "y();\n"
        digest = pair_hash(
            normalised([t for t, _ in tokenize(original)]),
            normalised([t for t, _ in tokenize(twin)]),
        )
        gate = '#[cfg(not(feature = "dictation-shadow"))]'
        feature = '#[cfg(feature = "dictation-shadow")]'
        branch = f"fn f() {{\n{gate}\n{original}{feature}\n{twin}}}\n"
        for pair, expected in [(digest, 0), ("f" * 16, 1)]:
            entry = {"first": "x();", "form": "frozen", "pair": pair, "why": "x"}
            with allowed({LIVE: [entry]}):
                code, _, err = run(repo(f"fn f() {{\n{original}}}\n", branch))
            self.assertEqual(code, expected, err)
        # A trailing comma is rustfmt's, not a change of the pair.
        twin_comma = "y(a,);\n"
        twin_plain = "y(a);\n"
        self.assertEqual(
            pair_hash(["x"], normalised([t for t, _ in tokenize(twin_comma)])),
            pair_hash(["x"], normalised([t for t, _ in tokenize(twin_plain)])),
        )

    def test_the_named_attribute_is_accepted_once_and_no_other_is(self):
        from scripts.check_shadow_diff import SHADOW_ATTR_ALLOW

        self.assertEqual(
            [e["attr"] for entries in SHADOW_ATTR_ALLOW.values() for e in entries],
            ["#[track_caller]", "#[track_caller]"],
        )
        item = "fn lock(&self);\n"
        entry = {"attr": "#[track_caller]", "first": "fn lock(&self);", "why": "caller location"}
        from unittest import mock

        import scripts.check_shadow_diff as module

        with mock.patch.dict(module.SHADOW_ATTR_ALLOW, {LIVE: [entry]}, clear=True):
            code, _, err = run(repo(item, f"#[track_caller]\n{item}"))
            self.assertEqual(code, 0, err)
            for other in ["#[inline]", "#[must_use]", "#[track_caller] #[inline]"]:
                code, _, err = run(repo(item, f"{other}\n{item}"))
                self.assertEqual(code, 1, f"{other}: {err}")
            # The same attribute twice, or on a statement that is not named.
            code, _, err = run(repo(item, f"#[track_caller]\n#[track_caller]\n{item}"))
            self.assertEqual(code, 1, err)
            code, _, err = run(repo("fn other(&self);\n", "#[track_caller]\nfn other(&self);\n"))
            self.assertEqual(code, 1, err)

    def test_adding_cfg_test_to_an_existing_production_file_does_not_exempt_it(self):
        root = repo("fn a() {}\nfn b() {}\n", "#![cfg(test)]\nfn a() {}\n")
        code, _, err = run(root)
        self.assertEqual(code, 1, err)
        self.assertIn("removes", err)

    def test_a_trailing_comment_does_not_hide_an_always_false_gate(self):
        for gate in [
            "#[cfg(any())] // keep",
            "#[cfg(any())] /* keep */",
            "#[cfg(not(all()))]// explain",
            "#[cfg(\n    any()\n)]",
        ]:
            root = repo("a\nb\nc\n", f"a\n{gate}\nb\nc\n")
            code, _, err = run(root)
            self.assertEqual(code, 1, f"{gate!r}: {err}")
            self.assertIn("is not the one permitted gate", err)

    def test_a_gate_that_holds_only_in_a_test_build_disables_the_existing_statement(self):
        for gate in ["#[cfg(test)]", "#[cfg(all(test, unix))]"]:
            root = repo("a\nb\nc\n", f"a\n{gate}\nb\nc\n")
            code, _, err = run(root)
            self.assertEqual(code, 1, f"{gate}: {err}")

    def test_a_string_with_slashes_is_not_a_comment(self):
        # The `//` inside the string must not swallow the rest of the attribute: the gate is
        # seen, and it is not the permitted one.
        root = repo(
            "a\nb\n",
            'a\n#[cfg(feature = "x//y")]\nnew();\n#[cfg(not(feature = "x//y"))]\nb\n',
        )
        code, _, err = run(root)
        self.assertEqual(code, 1, err)
        self.assertIn('feature="x//y"', err)

    def test_a_file_level_gate_added_to_a_production_file_fails_by_itself(self):
        for gate in ["#![cfg(test)]", "#![cfg(feature = \"dictation-shadow\")]", "#![cfg(any())]"]:
            root = repo("fn a() {}\nfn b() {}\n", f"{gate}\nfn a() {{}}\nfn b() {{}}\n")
            code, _, err = run(root)
            self.assertEqual(code, 1, f"{gate}: {err}")
            self.assertIn("gates the whole of a file the base already had", err)

    def test_a_file_level_gate_in_a_new_file_is_harmless(self):
        root = repo("a\n", "a\nb\n")
        path = "app/src-tauri/src/commands/recording_extra.rs"
        (root / path).parent.mkdir(parents=True, exist_ok=True)
        (root / path).write_text("#![cfg(feature = \"dictation-shadow\")]\nfn a() {}\n")
        git(root, "add", ".")
        git(root, "commit", "-m", "a new gated file")
        code, _, err = run(root)
        self.assertEqual(code, 0, err)

    def test_numstat_reads_added_and_removed(self):
        root = repo("a\n", "a\nb\n")
        self.assertEqual(numstat(root, "main")[LIVE], (1, 0))


if __name__ == "__main__":
    unittest.main()
