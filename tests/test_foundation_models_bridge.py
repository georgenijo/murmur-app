"""Exercise one-use session ownership without loading FoundationModels or an app."""

from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


@unittest.skipUnless(sys.platform == "darwin" and shutil.which("xcrun"), "macOS Swift required")
class PreparedSessionTests(unittest.TestCase):
    def test_take_release_and_stale_publication(self):
        bridge = Path(__file__).resolve().parents[1] / "app/src-tauri/macos/FoundationModelsBridge.swift"
        assertions = r'''
private let first = PreparedSession<String>()
private let second = PreparedSession<String>()
first.publish("first-session")
second.publish("second-session")
assert(first.take() == "first-session")
assert(first.take() == nil)
first.publish("late-result")
assert(first.take() == nil)
assert(second.take() == "second-session")

private let cancelled = PreparedSession<String>()
cancelled.invalidate()
cancelled.publish("stale-result")
assert(cancelled.take() == nil)
private let readyThenCancelled = PreparedSession<String>()
readyThenCancelled.publish("unused-session")
readyThenCancelled.invalidate()
assert(readyThenCancelled.take() == nil)

private let pending = PreparedSession<String>()
assert(pending.take() == nil)
pending.publish("too-late")
assert(pending.take() == nil)

private let concurrent = PreparedSession<String>()
concurrent.publish("single-use")
let countLock = NSLock()
var winners = 0
DispatchQueue.concurrentPerform(iterations: 32) { _ in
    if concurrent.take() != nil {
        countLock.lock()
        winners += 1
        countLock.unlock()
    }
}
assert(winners == 1)
'''
        with tempfile.TemporaryDirectory(prefix="murmur-fm-unit-") as temp:
            source = Path(temp) / "main.swift"
            binary = Path(temp) / "session-tests"
            source.write_text(bridge.read_text() + assertions)
            compiled = subprocess.run(
                ["xcrun", "swiftc", "-D", "MURMUR_FM_STUB", str(source), "-o", str(binary)],
                capture_output=True, text=True, timeout=90,
            )
            self.assertEqual(compiled.returncode, 0, compiled.stderr)
            subprocess.run([str(binary)], check=True, capture_output=True, text=True, timeout=10)


if __name__ == "__main__":
    unittest.main()
