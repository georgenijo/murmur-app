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
            self.assertIn("disables the existing statement", err)

    def test_an_always_false_gate_on_a_new_statement_is_harmless(self):
        root = repo("a\nb\n", "a\n#[cfg(any())]\nnew();\nb\n")
        code, _, err = run(root)
        self.assertEqual(code, 0, err)

    def test_a_real_gate_on_an_existing_statement_passes(self):
        root = repo(
            "a\nb\n",
            "a\n#[cfg(not(feature = \"dictation-shadow\"))]\nb\n"
            "#[cfg(feature = \"dictation-shadow\")]\nb2();\n",
        )
        code, _, err = run(root)
        self.assertEqual(code, 0, err)

    def test_adding_cfg_test_to_an_existing_production_file_does_not_exempt_it(self):
        root = repo("fn a() {}\nfn b() {}\n", "#![cfg(test)]\nfn a() {}\n")
        code, _, err = run(root)
        self.assertEqual(code, 1, err)
        self.assertIn("removes", err)

    def test_numstat_reads_added_and_removed(self):
        root = repo("a\n", "a\nb\n")
        self.assertEqual(numstat(root, "main")[LIVE], (1, 0))


if __name__ == "__main__":
    unittest.main()
