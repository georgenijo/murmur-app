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

    def test_the_main_thread_watch_allowance_applies_only_to_a_shadow_pr(self):
        path = "app/src-tauri/src/main_thread_watch.rs"
        for marker, expected in [("", 0), (SHADOW_MARK, 1)]:
            root = repo("a();\n", "a();\nb();\n")
            (root / path).parent.mkdir(parents=True, exist_ok=True)
            (root / path).write_text("p();\nq();\nr();\ns();\n")
            git(root, "add", ".")
            git(root, "commit", "-m", "watch on main")
            git(root, "checkout", "-q", "main")
            git(root, "merge", "-q", "--ff-only", "branch")
            git(root, "checkout", "-q", "-b", "edit")
            (root / path).write_text(marker + "p();\ns();\n")
            git(root, "commit", "-am", "remove two lines")
            code, _, err = run(root)
            self.assertEqual(code, expected, f"{marker!r}: {err}")
            if expected:
                self.assertIn("removes 2 lines", err)

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


if __name__ == "__main__":
    unittest.main()
