"""D-S2c-15 C2 and C3: the scoped `check_shadow_diff.py`.

The insertion-only and zero-deletion rules bind only a PR that adds or removes lines under a
`dictation-shadow` gate. Twin consistency, feature scoping and no new twins bind every PR
that changes a production file.
"""

import unittest

from tests.test_shadow_diff import LIVE, SHADOW_MARK, allowed, git, repo, run, shadow_repo


class ScopedCheckTests(unittest.TestCase):
    # ------------------------------------------------------------ scoping of the rules

    def test_a_pr_that_changes_no_shadow_line_may_edit_and_delete_live_code(self):
        for base, branch in [
            ("a();\nb();\nc();\n", "a();\nc();\n"),
            ("a();\nb();\n", "a();\nb2();\n"),
            ("a();\nb();\nc();\n", "b();\na();\nc();\n"),
            ("a();\nb();\n", "a();\nb();\nreturn;\n"),
            ("a();\nb();\n", "#[cfg(any())]\na();\nb();\n"),
        ]:
            code, out, err = run(repo(base, branch))
            self.assertEqual(code, 0, f"{branch!r}: {err}")
            self.assertIn("shadow change: no", out)

    def test_a_shadow_pr_is_insertion_only_again(self):
        for base, branch in [
            ("a();\nb();\nc();\n", "a();\nc();\n"),
            ("a();\nb();\n", "a();\nb2();\n"),
        ]:
            code, out, err = run(shadow_repo(base, branch))
            self.assertEqual(code, 1, f"{branch!r}: {err}")
            self.assertIn("shadow change: yes", out)
            self.assertIn("removes", err)

    def test_removing_a_shadow_line_is_a_shadow_change(self):
        root = repo(SHADOW_MARK + "a();\nb();\n", "a();\nb();\n")
        code, out, err = run(root)
        self.assertIn("shadow change: yes", out)
        self.assertIn("(removed)", out)
        self.assertEqual(code, 1, err)

    WATCH = "app/src-tauri/src/main_thread_watch.rs"
    EXACT = 'let end = start + lib_rs[start..].find(\']\').expect("handler end");'
    REPLACED = 'let end = start + lib_rs[start..].find("]))").expect("handler end");'

    def watch_repo(self, branch_text: str):
        base_text = (
            "fn instrument_invoke_handler() {\n    inner();\n    more();\n}\n"
            "#[cfg(test)]\nmod tests {\n    fn parse() {\n        " + self.EXACT + "\n    }\n}\n"
        )
        root = repo("a();\n", "a();\nb();\n")
        git(root, "checkout", "-q", "main")
        (root / self.WATCH).write_text(base_text)
        git(root, "add", ".")
        git(root, "commit", "-m", "watch on main")
        git(root, "checkout", "-q", "-B", "edit")
        (root / self.WATCH).write_text(branch_text)
        git(root, "commit", "-am", "edit the watch")
        return root

    def watch_text(self, *, inserted: str = "", removed_live: bool = False, replaced: str | None = None) -> str:
        replaced = self.REPLACED if replaced is None else replaced
        live = "    more();\n" if removed_live else "    inner();\n    more();\n"
        return (
            "fn instrument_invoke_handler() {\n" + inserted + live + "}\n"
            "#[cfg(test)]\nmod tests {\n    fn parse() {\n        " + replaced + "\n    }\n}\n"
        )

    def test_the_main_thread_watch_allowance_is_the_one_named_line_and_only_in_a_shadow_pr(self):
        # The exact parser-test replacement alone: a PR with no shadow line is ordinary, and a
        # shadow PR (a gated marker elsewhere) may remove that one line.
        code, out, err = run(self.watch_repo(self.watch_text()))
        self.assertEqual(code, 0, err)
        self.assertIn("shadow change: no", out)
        root = self.watch_repo(self.watch_text() + SHADOW_MARK)
        code, out, err = run(root)
        self.assertEqual(code, 0, err)
        self.assertIn("shadow change: yes", out)

    def test_an_ungated_return_in_the_invoke_handler_instrumentation_fails_in_a_shadow_pr(self):
        # Sol's plant: the allowance must not exempt the rest of the file.
        for inserted in ["    return false;\n", "    if flag {\n        return;\n    }\n"]:
            root = self.watch_repo(self.watch_text(inserted=inserted) + SHADOW_MARK)
            code, _, err = run(root)
            self.assertEqual(code, 1, f"{inserted!r}: {err}")
            self.assertIn("not behind the dictation-shadow feature", err)

    SHADOW_MODULE = "app/src-tauri/src/commands/recording/shadow_commands.rs"

    def with_shadow_module_edit(self, root):
        """Also edit a feature-only module, with no gated line anywhere else (Sol round 2, 4)."""
        git(root, "checkout", "-q", "main")
        module = root / self.SHADOW_MODULE
        module.parent.mkdir(parents=True, exist_ok=True)
        module.write_text("fn witnessed() {}\n")
        git(root, "add", ".")
        git(root, "commit", "-m", "module on main")
        git(root, "checkout", "-q", "edit")
        git(root, "merge", "-q", "main", "-m", "merge main")
        module.write_text("fn witnessed() {}\nfn another() {}\n")
        git(root, "commit", "-am", "edit the feature-only module")
        return root

    def test_an_edit_in_a_feature_only_module_makes_the_pr_a_shadow_change(self):
        # Sol's plant: the module edit beside an ungated `return false` in the instrumentation.
        root = self.with_shadow_module_edit(self.watch_repo(self.watch_text(inserted="    return false;\n")))
        code, out, err = run(root)
        self.assertIn("shadow change: yes", out)
        self.assertIn("feature-only module changed", out)
        self.assertEqual(code, 1, err)
        self.assertIn("not behind the dictation-shadow feature", err)

    def test_a_feature_only_module_edit_alone_stays_exempt_from_insertion_only(self):
        root = self.with_shadow_module_edit(self.watch_repo(self.watch_text()))
        code, out, err = run(root)
        self.assertIn("shadow change: yes", out)
        self.assertEqual(code, 0, err)

    def test_removing_any_other_line_of_the_watch_file_fails_in_a_shadow_pr(self):
        root = self.watch_repo(self.watch_text(removed_live=True) + SHADOW_MARK)
        code, _, err = run(root)
        self.assertEqual(code, 1, err)
        self.assertIn("not the line SHADOW_DIFF_ALLOW names", err)

    def test_a_gated_wrapper_around_the_live_code_of_the_watch_file_fails_in_a_shadow_pr(self):
        # Zero removed live lines, but the base's `inner();` is hidden in the default build.
        text = self.watch_text().replace(
            "    inner();\n", '    #[cfg(feature = "dictation-shadow")]\n    {\n        inner();\n    }\n'
        )
        code, _, err = run(self.watch_repo(text + SHADOW_MARK))
        self.assertEqual(code, 1, err)

    # ----------------------------------------------------------- feature scoping (C2 i)

    LIVE_FN = "fn live() {\n    first();\n    second();\n}\n"

    def planted(self, inserted: str, *, shadow: bool = True):
        branch = f"fn live() {{\n    first();\n{inserted}    second();\n}}\n"
        return (shadow_repo if shadow else repo)(self.LIVE_FN, branch)

    def test_an_unguarded_inserted_early_return_fails_in_a_shadow_change(self):
        # Sol's plant: zero deleted lines, so the old rule let it through.
        for inserted in [
            "    return;\n",
            "    if flag {\n        return;\n    }\n",
            "    if !crate::dictation_shadow::enabled() {\n        return;\n    }\n",
            "    other::note();\n",
        ]:
            code, _, err = run(self.planted(inserted))
            self.assertEqual(code, 1, f"{inserted!r}: {err}")
            self.assertIn("not behind the dictation-shadow feature", err)

    def test_the_same_early_return_is_an_ordinary_change_in_a_pr_with_no_shadow_line(self):
        code, out, err = run(self.planted("    return;\n", shadow=False))
        self.assertEqual(code, 0, err)
        self.assertIn("shadow change: no", out)

    def test_an_inserted_statement_behind_the_feature_passes(self):
        feature = '    #[cfg(feature = "dictation-shadow")]\n'
        for inserted in [
            feature + "    note();\n",
            feature + "    {\n        if flight.is_active() {\n            return;\n        }\n    }\n",
            "    crate::shadow_site!(issue = 1154);\n",
            "    if crate::dictation_shadow::enabled() {\n        note();\n    }\n",
        ]:
            code, _, err = run(self.planted(inserted))
            self.assertEqual(code, 0, f"{inserted!r}: {err}")

    def test_the_enable_flag_block_may_not_carry_an_else_branch(self):
        code, _, err = run(
            self.planted(
                "    if crate::dictation_shadow::enabled() {\n        note();\n    } else {\n        other();\n    }\n"
            )
        )
        self.assertEqual(code, 1, err)

    def test_a_gate_that_is_not_off_by_the_feature_alone_is_not_behind_it(self):
        for gate in [
            '#[cfg(any(feature = "dictation-shadow", unix))]',
            '#[cfg(feature = r"dictation-shadow")]',
        ]:
            code, _, err = run(self.planted(f"    {gate}\n    note();\n"))
            self.assertEqual(code, 1, f"{gate}: {err}")
            self.assertIn("not behind the dictation-shadow feature", err)

    def test_a_new_file_is_not_an_inserted_statement(self):
        root = shadow_repo(self.LIVE_FN, self.LIVE_FN)
        path = "app/src-tauri/src/commands/recording_probe.rs"
        (root / path).parent.mkdir(parents=True, exist_ok=True)
        (root / path).write_text("fn new_helper() {\n    work();\n}\n")
        git(root, "add", ".")
        git(root, "commit", "-m", "a new file")
        code, _, err = run(root)
        self.assertEqual(code, 0, err)

    # ------------------------------------------------------ twin consistency (C2 ii)

    GATE = '#[cfg(not(feature = "dictation-shadow"))]'
    FEATURE = '#[cfg(feature = "dictation-shadow")]'

    def twin_file(self, original: str, twin: str) -> str:
        return f"fn f() {{\n{self.GATE}\n{original}{self.FEATURE}\n{twin}}}\n"

    def test_an_original_edited_without_its_twin_fails_in_any_pr(self):
        old, new = "go(a, b);\n", "go(a, b, c);\n"
        with allowed({LIVE: [{"first": "go(a, b);", "form": "closed", "why": "twin"}]}):
            root = repo(self.twin_file(old, old), self.twin_file(new, old))
        # The edit renames the statement the table names, as the PR making it would.
        with allowed({LIVE: [{"first": "go(a, b, c);", "form": "closed", "why": "twin"}]}):
            code, out, err = run(root)
        self.assertIn("shadow change: no", out)
        self.assertEqual(code, 1, err)
        self.assertIn("differs from the original", err)
        self.assertIn("The original changed but its twin did not", err)
        self.assertIn("an original and its twin change together", err)

    def test_an_original_and_its_twin_edited_together_pass(self):
        old, new = "go(a, b);\n", "go(a, b, c);\n"
        with allowed({LIVE: [{"first": "go(a, b);", "form": "closed", "why": "twin"}]}):
            root = repo(self.twin_file(old, old), self.twin_file(new, new))
        with allowed({LIVE: [{"first": "go(a, b, c);", "form": "closed", "why": "twin"}]}):
            code, out, err = run(root)
        self.assertEqual(code, 0, err)
        self.assertIn("shadow change: no", out)

    def test_a_frozen_pair_needs_its_new_digest_pinned(self):
        from scripts.check_shadow_diff import normalised, pair_hash, tokenize

        def digest(original, twin):
            return pair_hash(
                normalised([t for t, _ in tokenize(original)]),
                normalised([t for t, _ in tokenize(twin)]),
            )

        old, new = "keep(a);\n", "keep(a, b);\n"
        reviewed = digest(old, "see(a);\n")

        def entry(first, pair):
            return {"first": first, "form": "frozen", "pair": pair, "why": "x"}

        with allowed({LIVE: [entry("keep(a);", reviewed)]}):
            root = repo(self.twin_file(old, "see(a);\n"), self.twin_file(new, "see(a, b);\n"))
        # Both sides edited, the digest not re-pinned: fails, and says what to do.
        with allowed({LIVE: [entry("keep(a, b);", reviewed)]}):
            code, _, err = run(root)
        self.assertEqual(code, 1, err)
        self.assertIn("is not the pair a reviewer accepted", err)
        self.assertIn("pin the new digest", err)
        self.assertIn("Both changed", err)
        # Re-pinned by a reviewer: passes.
        with allowed({LIVE: [entry("keep(a, b);", digest(new, "see(a, b);\n"))]}):
            code, _, err = run(root)
        self.assertEqual(code, 0, err)

    def test_a_twin_entry_with_no_twin_left_in_the_file_fails(self):
        with allowed({LIVE: [{"first": "go(a);", "form": "closed", "why": "twin"}]}):
            code, _, err = run(repo("go(a);\n", "go(a);\nother();\n"))
        self.assertEqual(code, 1, err)
        self.assertIn("has no", err)

    # ------------------------------------------------------------ no new twins (C2 iii)

    def test_a_new_twin_fails_whatever_its_spelling(self):
        feature = 'feature = "dictation-shadow"'
        live = "fn live() {\n    first();\n}\n"
        for gate in [
            self.GATE,
            f"#[cfg(all(not({feature}), unix))]",
            f'#[cfg(any(not({feature}), target_os = "linux"))]',
            '#[cfg(not(feature = r"dictation-shadow"))]',
            f"#[cfg_attr(not({feature}), inline)]",
        ]:
            code, _, err = run(repo(live, f"{live}{gate}\nfn extra() {{}}\n"))
            self.assertEqual(code, 1, f"{gate}: {err}")
            self.assertTrue(
                "is not in SHADOW_TWINS" in err or "switches an item off in the feature build" in err,
                f"{gate}: {err}",
            )

    # -------------------------------------------------- pinned copies of live functions (C3)

    PINNED = (
        "fn orig(x: u64) -> bool {\n    x > 1\n}\n\n"
        '#[cfg(feature = "dictation-shadow")]\n'
        "fn orig_observed(x: u64) -> (bool, u64) {\n    (x > 1, x)\n}\n"
    )

    def pin(self, text: str) -> dict:
        from scripts.check_shadow_diff import function_tokens, pair_hash, tokenize

        tokens, lines = tokenize(text), text.splitlines()
        digest = pair_hash(
            function_tokens(tokens, lines, "fn orig(x: u64) -> bool {"),
            function_tokens(tokens, lines, "fn orig_observed(x: u64) -> (bool, u64) {"),
        )
        return {
            "first": "fn orig_observed(x: u64) -> (bool, u64) {",
            "original": "fn orig(x: u64) -> bool {",
            "form": "frozen_fn",
            "pair": digest,
            "why": "copy",
        }

    def test_an_edit_that_leaves_the_pin_stale_fails_in_any_pr(self):
        entry = self.pin(self.PINNED)
        for text, hint in [
            (self.PINNED.replace("x > 1\n}", "x > 2\n}", 1), "the original changed but its observed copy did not"),
            (self.PINNED.replace("(x > 1, x)", "(x > 1, x + 1)"), "the observed copy changed but its original did not"),
        ]:
            with allowed({LIVE: [entry]}):
                code, _, err = run(repo(self.PINNED, text))
            self.assertEqual(code, 1, err)
            self.assertIn("is no longer the pinned copy", err)
            self.assertIn(hint, err)
            self.assertIn("pin the new digest", err)

    def test_an_unedited_pin_passes_and_a_re_pinned_edit_passes(self):
        with allowed({LIVE: [self.pin(self.PINNED)]}):
            code, _, err = run(repo(self.PINNED, self.PINNED + "fn other() {}\n"))
        self.assertEqual(code, 0, err)
        edited = self.PINNED.replace("x > 1\n}", "x > 2\n}", 1).replace("(x > 1, x)", "(x > 2, x)")
        with allowed({LIVE: [self.pin(edited)]}):
            code, _, err = run(repo(self.PINNED, edited))
        self.assertEqual(code, 0, err)

    def test_a_pinned_copy_missing_from_the_file_fails(self):
        with allowed({LIVE: [self.pin(self.PINNED)]}):
            code, _, err = run(repo(self.PINNED, "fn orig(x: u64) -> bool {\n    x > 1\n}\n"))
        self.assertEqual(code, 1, err)
        self.assertIn("is not in the file", err)

    def test_the_real_table_pins_every_observed_copy_of_a_live_function(self):
        from scripts.check_shadow_diff import ROOT, SHADOW_TWINS, twin_problems

        pinned = {
            entry["first"].split("fn ", 1)[1].split("(", 1)[0].split("<", 1)[0]
            for entries in SHADOW_TWINS.values()
            for entry in entries
            if entry["form"] == "frozen_fn"
        }
        self.assertEqual(
            pinned,
            {
                "set_active_context_observed",
                "clear_active_context_observed",
                "cancel_recording_observed",
                "register_partial_ticker_locked_observed",
                "note_ticker_started_observed",
                "ticker_started_observed",
                "consume_observed",
                "release_and_reply_observed",
                "detached_observed",
                "detached_under_d_observed",
                "cancel_through_observed",
                "set_context_observed",
                "clear_context_observed",
            },
        )
        # Every entry of the table holds against the committed tree.
        for path in SHADOW_TWINS:
            self.assertEqual(twin_problems(ROOT, "HEAD", path), [], path)

    def test_a_frozen_fn_entry_names_its_original_and_a_digest(self):
        from scripts.check_shadow_diff import twin_table_problems

        base = {"first": "fn b() {", "form": "frozen_fn", "why": "x", "pair": "1" * 16}
        self.assertTrue(twin_table_problems({LIVE: [base]}))
        self.assertEqual(twin_table_problems({LIVE: [{**base, "original": "fn a() {"}]}), [])
        self.assertTrue(twin_table_problems({LIVE: [{**base, "original": "fn a() {", "pair": ""}]}))


class HandlerAndBaseTests(unittest.TestCase):
    """Sol round 1 on #1174: the handler pair, a moving base, and a clean retirement."""

    GATE = '#[cfg(not(feature = "dictation-shadow"))]'
    FEATURE = '#[cfg(feature = "dictation-shadow")]'
    ENTRY = {
        "first": "commands::recording::start_native_recording,",
        "form": "handler",
        "twin": "commands::recording::shadow_commands::start_native_recording_witnessed,",
        "why": "the witnessing wrapper of the command",
    }

    def handler_text(self, original: str, twin: str) -> str:
        return f"a,\n{self.GATE}\n{original}\n{self.FEATURE}\n{twin}\nb,\n"

    def test_the_feature_entry_must_be_the_witnessing_wrapper(self):
        base = "a,\ncommands::recording::start_native_recording,\nb,\n"
        original = self.ENTRY["first"]
        good = self.ENTRY["twin"]
        with allowed({LIVE: [self.ENTRY]}):
            code, _, err = run(repo(base, self.handler_text(original, good)))
            self.assertEqual(code, 0, err)
            # Sol's plant: the feature entry is the ordinary command, so the reply is never
            # witnessed. The old check read the attribute stack only.
            code, _, err = run(repo(base, self.handler_text(original, original)))
            self.assertEqual(code, 1, err)
            self.assertIn("must register the witnessing wrapper", err)
            # Another command, another wrapper, or none at all.
            for twin in [
                "commands::recording::shadow_commands::stop_native_recording_witnessed,",
                "commands::recording::shadow_commands::start_native_recording_witnessed",
            ]:
                code, _, err = run(repo(base, self.handler_text(original, twin)))
                self.assertEqual(code, 1, f"{twin}: {err}")
            code, _, err = run(repo(base, f"a,\n{self.GATE}\n{original}\nb,\n"))
            self.assertEqual(code, 1, err)
            self.assertIn("has no feature entry", err)
            # The entry itself is pinned too.
            code, _, err = run(repo(base, self.handler_text("commands::recording::other,", good)))
            self.assertEqual(code, 1, err)

    def test_the_real_handler_entries_name_their_wrappers(self):
        from scripts.check_shadow_diff import SHADOW_TWINS, twin_table_problems

        entries = [e for e in SHADOW_TWINS["app/src-tauri/src/lib.rs"] if e["form"] == "handler"]
        self.assertEqual(len(entries), 4)
        for entry in entries:
            name = entry["first"].rsplit("::", 1)[1]
            self.assertEqual(entry["twin"], f"commands::recording::shadow_commands::{name[:-1]}_witnessed,")
        broken = [{k: v for k, v in e.items() if k != "twin"} for e in entries]
        self.assertTrue(twin_table_problems({"app/src-tauri/src/lib.rs": broken}))

    def test_a_base_branch_that_advanced_does_not_hide_a_removed_shadow_line(self):
        base = f"a();\n{self.FEATURE}\nshadow_note();\nb();\n"
        root = repo(base, f"a();\n{self.FEATURE}\nb();\n")  # the capture call is removed
        code, out, err = run(root)
        self.assertEqual(code, 1, err)
        self.assertIn("shadow change: yes", out)
        # `main` then gains three unrelated lines above: the removed line's number, looked up in
        # the new tip, is now another line. The merge base is resolved once.
        git(root, "checkout", "-q", "main")
        (root / LIVE).write_text("x1();\nx2();\nx3();\n" + base)
        git(root, "commit", "-am", "main moves on")
        git(root, "checkout", "-q", "branch")
        code, out, err = run(root)
        self.assertEqual(code, 1, err)
        self.assertIn("shadow change: yes", out)
        self.assertIn("removes", err)

    def test_the_diff_reads_the_merge_base_not_the_moved_tip(self):
        from scripts.check_shadow_diff import resolve_base

        root = repo("a();\nb();\n", "a();\nb();\nc();\n")
        merge_base = resolve_base(root, "main")
        git(root, "checkout", "-q", "main")
        (root / LIVE).write_text("z();\na();\nb();\n")
        git(root, "commit", "-am", "main moves on")
        git(root, "checkout", "-q", "branch")
        self.assertEqual(resolve_base(root, "main"), merge_base)

    # ------------------------------------------------------------------ a clean retirement

    SCRIPT = (
        "SHADOW_TWINS: dict[str, list[dict]] = {\n"
        '    "app/src-tauri/src/keyboard.rs": [\n'
        '        {"first": "fn orig_observed(x: u64) -> (bool, u64) {", "form": "frozen_fn",\n'
        '         "original": "fn orig(x: u64) -> bool {", "pair": "0000000000000000", "why": "x"},\n'
        "    ],\n}\n"
    )

    def retirement_repo(self, base_text: str, branch_text: str):
        root = repo(base_text, branch_text)
        git(root, "checkout", "-q", "main")
        (root / "scripts").mkdir(exist_ok=True)
        (root / "scripts/check_shadow_diff.py").write_text(self.SCRIPT)
        git(root, "add", ".")
        git(root, "commit", "-m", "the reviewed table")
        git(root, "checkout", "-q", "branch")
        git(root, "merge", "-q", "main", "-m", "merge main")
        return root

    BASE = (
        "fn orig(x: u64) -> bool {\n    x > 1\n}\n"
        '#[cfg(feature = "dictation-shadow")]\n'
        "fn orig_observed(x: u64) -> (bool, u64) {\n    (x > 1, x)\n}\n"
        "fn caller() {\n"
        '    #[cfg(not(feature = "dictation-shadow"))]\n'
        "    let a = orig(1);\n"
        '    #[cfg(feature = "dictation-shadow")]\n'
        "    let (a, witness) = orig_observed(1);\n"
        "    use_it(a);\n}\n"
    )

    def test_a_complete_retirement_of_a_pinned_copy_and_its_call_sites_passes(self):
        # The cutover removes the observed copy with its feature gate, the call site's gates and
        # twin, and re-points the original: its table entry goes with it.
        branch = (
            "fn orig(x: u64) -> bool {\n    x > 1\n}\n"
            "fn caller() {\n    let a = decide(1);\n    use_it(a);\n}\n"
        )
        with allowed({}):
            code, out, err = run(self.retirement_repo(self.BASE, branch))
        self.assertEqual(code, 0, err)
        self.assertIn("shadow change: no", out)

    def test_removing_the_copy_but_leaving_its_pin_fails(self):
        branch = self.BASE.replace(
            '#[cfg(feature = "dictation-shadow")]\nfn orig_observed(x: u64) -> (bool, u64) {\n    (x > 1, x)\n}\n', ""
        )
        pin = {
            "first": "fn orig_observed(x: u64) -> (bool, u64) {",
            "form": "frozen_fn",
            "original": "fn orig(x: u64) -> bool {",
            "pair": "0" * 16,
            "why": "x",
        }
        with allowed({LIVE: [pin]}):
            code, _, err = run(self.retirement_repo(self.BASE, branch))
        self.assertEqual(code, 1, err)
        self.assertIn("is not in the file", err)

    def test_removing_the_pin_but_leaving_a_capture_line_is_still_a_shadow_change(self):
        # Only the table's own copies are retired: any other shadow line that goes is a removal.
        base = self.BASE + '#[cfg(feature = "dictation-shadow")]\nfn capture() {}\n'
        with allowed({}):
            code, out, err = run(self.retirement_repo(base, self.BASE))
        self.assertEqual(code, 1, err)
        self.assertIn("shadow change: yes", out)


    def test_removing_a_pin_while_the_copy_and_its_callers_remain_fails(self):
        # Sol's plant: the pin goes, the observed copy and its call site stay, and the original
        # changes (its refusal reply differs from the copy's).
        branch = self.BASE.replace("x > 1\n}\n#", "x > 2\n}\n#", 1)
        with allowed({}):
            code, _, err = run(self.retirement_repo(self.BASE, branch))
        self.assertEqual(code, 1, err)
        self.assertIn("was removed from SHADOW_TWINS", err)
        self.assertIn("orig_observed", err)

    def test_removing_a_pin_with_the_copy_but_a_call_site_left_fails(self):
        branch = self.BASE.replace(
            '#[cfg(feature = "dictation-shadow")]\nfn orig_observed(x: u64) -> (bool, u64) {\n    (x > 1, x)\n}\n', ""
        )
        with allowed({}):
            code, _, err = run(self.retirement_repo(self.BASE, branch))
        self.assertEqual(code, 1, err)
        self.assertIn("call site remains", err)


class LifecycleCutoverTests(unittest.TestCase):
    """Sol round 3, finding 3: a cutover removes a handler pair, its wrapper in the
    feature-only `shadow_commands.rs`, its pins and its call sites, in one PR. It must pass;
    an unrelated edit of the same module beside it must still count as a shadow change."""

    GATE = '#[cfg(not(feature = "dictation-shadow"))]'
    FEATURE = '#[cfg(feature = "dictation-shadow")]'
    LIB = "app/src-tauri/src/lib.rs"
    MODULE = "app/src-tauri/src/commands/recording/shadow_commands.rs"
    SCRIPT = (
        "SHADOW_TWINS: dict[str, list[dict]] = {\n"
        '    "app/src-tauri/src/lib.rs": [\n'
        '        {"first": "commands::recording::process_audio,", "form": "handler",\n'
        '         "twin": "commands::recording::shadow_commands::process_audio_witnessed,", "why": "x"},\n'
        "    ],\n"
        '    "app/src-tauri/src/keyboard.rs": [\n'
        '        {"first": "fn orig_observed(x: u64) -> (bool, u64) {", "form": "frozen_fn",\n'
        '         "original": "fn orig(x: u64) -> bool {", "pair": "0000000000000000", "why": "x"},\n'
        "    ],\n}\n"
    )
    LIVE_BASE = (
        "fn orig(x: u64) -> bool {\n    x > 1\n}\n"
        '#[cfg(feature = "dictation-shadow")]\n'
        "fn orig_observed(x: u64) -> (bool, u64) {\n    (x > 1, x)\n}\n"
        "fn caller() {\n"
        '    #[cfg(not(feature = "dictation-shadow"))]\n'
        "    let a = orig(1);\n"
        '    #[cfg(feature = "dictation-shadow")]\n'
        "    let (a, witness) = orig_observed(1);\n"
        "    use_it(a);\n}\n"
    )
    LIVE_AFTER = "fn orig(x: u64) -> bool {\n    x > 1\n}\nfn caller() {\n    let a = decide(1);\n    use_it(a);\n}\n"
    LIB_BASE = (
        "fn handlers() {\n    generate_handler![\n        a,\n"
        f"        {GATE}\n        commands::recording::process_audio,\n"
        f"        {FEATURE}\n        commands::recording::shadow_commands::process_audio_witnessed,\n"
        "        b,\n    ]\n}\n"
    )
    LIB_AFTER = "fn handlers() {\n    generate_handler![\n        a,\n        commands::recording::process_audio,\n        b,\n    ]\n}\n"
    MODULE_BASE = (
        "use crate::commands::recording::process_audio;\n\n"
        "#[tauri::command]\npub async fn process_audio_witnessed() {\n    process_audio().await\n}\n\n"
        "#[tauri::command]\npub async fn other_witnessed() {\n    other().await\n}\n"
    )
    MODULE_AFTER = "#[tauri::command]\npub async fn other_witnessed() {\n    other().await\n}\n"

    def cutover_repo(self, module_after: str):
        root = repo(self.LIVE_BASE, self.LIVE_AFTER)
        git(root, "checkout", "-q", "main")
        for rel, text in (
            ("scripts/check_shadow_diff.py", self.SCRIPT),
            (self.LIB, self.LIB_BASE),
            (self.MODULE, self.MODULE_BASE),
        ):
            (root / rel).parent.mkdir(parents=True, exist_ok=True)
            (root / rel).write_text(text)
        git(root, "add", ".")
        git(root, "commit", "-m", "the reviewed tree")
        git(root, "checkout", "-q", "branch")
        git(root, "merge", "-q", "main", "-m", "merge main")
        (root / self.LIB).write_text(self.LIB_AFTER)
        (root / self.MODULE).write_text(module_after)
        git(root, "commit", "-am", "the cutover")
        return root

    def test_a_cutover_removing_the_pair_the_wrapper_the_pin_and_the_call_sites_passes(self):
        with allowed({}):
            code, out, err = run(self.cutover_repo(self.MODULE_AFTER))
        self.assertEqual(code, 0, err)
        self.assertIn("shadow change: no", out)

    def test_an_unrelated_edit_of_the_module_beside_the_cutover_is_a_shadow_change(self):
        for after in (
            self.MODULE_AFTER + "pub async fn another() {}\n",
            self.MODULE_AFTER.replace("other().await", "other().await?"),
            self.MODULE_AFTER.replace("    other().await\n", ""),
        ):
            with allowed({}):
                code, out, err = run(self.cutover_repo(after))
            self.assertIn("shadow change: yes", out, after)
            self.assertIn("feature-only module changed", out)
            # The cutover's own deletions are held to insertion only again.
            self.assertEqual(code, 1, err)

    def test_removing_a_wrapper_that_is_still_registered_is_a_shadow_change(self):
        # The handler entry stays in the table (not retired): the wrapper's removal is an edit.
        entry = {
            "first": "commands::recording::process_audio,",
            "form": "handler",
            "twin": "commands::recording::shadow_commands::process_audio_witnessed,",
            "why": "x",
        }
        with allowed({self.LIB: [entry]}):
            code, out, err = run(self.cutover_repo(self.MODULE_AFTER))
        self.assertIn("shadow change: yes", out)
        self.assertEqual(code, 1, err)

    def test_a_wrapper_removed_while_the_name_is_still_referenced_is_a_shadow_change(self):
        live_after = self.LIVE_AFTER + "fn leftover() { process_audio_witnessed(); }\n"
        root = repo(self.LIVE_BASE, live_after)
        git(root, "checkout", "-q", "main")
        for rel, text in (
            ("scripts/check_shadow_diff.py", self.SCRIPT),
            (self.LIB, self.LIB_BASE),
            (self.MODULE, self.MODULE_BASE),
        ):
            (root / rel).parent.mkdir(parents=True, exist_ok=True)
            (root / rel).write_text(text)
        git(root, "add", ".")
        git(root, "commit", "-m", "the reviewed tree")
        git(root, "checkout", "-q", "branch")
        git(root, "merge", "-q", "main", "-m", "merge main")
        (root / self.LIB).write_text(self.LIB_AFTER)
        (root / self.MODULE).write_text(self.MODULE_AFTER)
        git(root, "commit", "-am", "the cutover")
        with allowed({}):
            code, out, err = run(root)
        self.assertIn("shadow change: yes", out)
        self.assertEqual(code, 1, err)


class PinnedHelperTests(unittest.TestCase):
    """Sol round 1, finding 4: the live helper copies are pinned, not only their call sites."""

    def test_the_real_refusal_helper_and_the_detached_ready_task_copies_are_pinned(self):
        from scripts.check_shadow_diff import SHADOW_TWINS

        import re

        def name(line: str) -> str:
            return re.search(r"fn (\w+)", line).group(1)

        pairs = {
            (path.rsplit("/", 1)[1], name(entry["original"]), name(entry["first"]))
            for path, entries in SHADOW_TWINS.items()
            for entry in entries
            if entry["form"] == "frozen_fn"
        }
        for want in [
            ("start_ports.rs", "release_and_reply", "release_and_reply_observed"),
            ("lifecycle_ports.rs", "detached", "detached_observed"),
            ("ready_ports.rs", "detached_under_d", "detached_under_d_observed"),
        ]:
            self.assertIn(want, pairs)

    def test_editing_the_original_refusal_helper_fails_until_it_is_re_pinned(self):
        original = (
            "pub(super) fn release_and_reply(a: u64, err: Option<String>) -> Result<u64, String> {\n"
            "    reply_of(a, err)\n}\n"
        )
        copy = (
            "#[cfg(feature = \"dictation-shadow\")]\n"
            "pub(super) fn release_and_reply_observed(a: u64, err: Option<String>) -> Result<u64, String> {\n"
            "    reply_of(a, err)\n}\n"
        )
        from scripts.check_shadow_diff import pair_hash, tokenize, function_tokens

        text = original + copy
        tokens, lines = tokenize(text), text.splitlines()
        digest = pair_hash(
            function_tokens(tokens, lines, "pub(super) fn release_and_reply(a: u64, err: Option<String>) -> Result<u64, String> {"),
            function_tokens(tokens, lines, "pub(super) fn release_and_reply_observed(a: u64, err: Option<String>) -> Result<u64, String> {"),
        )
        pin = {
            "first": "pub(super) fn release_and_reply_observed(a: u64, err: Option<String>) -> Result<u64, String> {",
            "form": "frozen_fn",
            "original": "pub(super) fn release_and_reply(a: u64, err: Option<String>) -> Result<u64, String> {",
            "pair": digest,
            "why": "the armed refusal helper",
        }
        with allowed({LIVE: [pin]}):
            code, _, err = run(repo(text, text + "fn other() {}\n"))
            self.assertEqual(code, 0, err)
            # The armed copy still returns the error, the original no longer does.
            edited = text.replace("    reply_of(a, err)\n}\n#[cfg", "    reply_of(a, None)\n}\n#[cfg", 1)
            self.assertNotEqual(edited, text)
            code, out, err = run(repo(text, edited))
            self.assertEqual(code, 1, err)
            self.assertIn("no longer the pinned copy", err)
            self.assertIn("shadow change: no", out)


if __name__ == "__main__":
    unittest.main()
