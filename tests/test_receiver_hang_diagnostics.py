"""Server side of Murmur's server-armed hang-diagnostics protocol.

Covers the `/ingest` reply-shape arming (docs/features/log-shipping.md,
"Server-armed hang diagnostics") and the `/bundle` upload endpoint added in
infra/log-receiver/murmur-logs-receiver.py. Both control files
(`diag-installs.txt`, `collect-now.txt`) are re-read fresh on every request —
never cached — so disarming an install takes effect on its very next request.
"""

from __future__ import annotations

import http.client
import importlib.util
import json
import tempfile
import threading
import unittest
from pathlib import Path
from unittest import mock

RECEIVER_PATH = (
    Path(__file__).resolve().parents[1]
    / "infra"
    / "log-receiver"
    / "murmur-logs-receiver.py"
)
SPEC = importlib.util.spec_from_file_location("murmur_logs_receiver_hang", RECEIVER_PATH)
assert SPEC and SPEC.loader
receiver = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(receiver)


def event(summary: str, *, timestamp: str, data: dict | None = None) -> dict:
    return {
        "timestamp": timestamp,
        "stream": "system",
        "level": "info",
        "summary": summary,
        "data": data or {},
    }


class CollectNowParsingTests(unittest.TestCase):
    """Direct, HTTP-free tests of the control-file parsing helpers."""

    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.original_root = receiver.ROOT
        self.addCleanup(setattr, receiver, "ROOT", self.original_root)
        receiver.ROOT = self.directory.name
        self.install_id = "12345678-abcd"

    def _write_collect_now(self, text: str) -> None:
        (Path(receiver.ROOT) / receiver.COLLECT_NOW_FILENAME).write_text(
            text, encoding="utf-8"
        )

    def _write_diag_installs(self, text: str) -> None:
        (Path(receiver.ROOT) / receiver.DIAG_INSTALLS_FILENAME).write_text(
            text, encoding="utf-8"
        )

    def test_absent_collect_now_file_is_zero(self) -> None:
        self.assertEqual(receiver._collect_now_epoch(self.install_id), 0)

    def test_valid_line_is_honored(self) -> None:
        self._write_collect_now(f"{self.install_id} 1785873094\n")
        self.assertEqual(receiver._collect_now_epoch(self.install_id), 1785873094)

    def test_last_matching_line_wins(self) -> None:
        self._write_collect_now(
            f"{self.install_id} 100\n"
            f"other-install 999999\n"
            f"{self.install_id} 200\n"
        )
        self.assertEqual(receiver._collect_now_epoch(self.install_id), 200)

    def test_malformed_line_is_skipped_without_clobbering_a_prior_valid_line(
        self,
    ) -> None:
        self._write_collect_now(
            f"{self.install_id} 200\n"
            f"{self.install_id} not-a-number\n"
            f"{self.install_id} only-one-field\n"
        )
        self.assertEqual(receiver._collect_now_epoch(self.install_id), 200)

    def test_epoch_is_scoped_to_install_id_case_insensitively(self) -> None:
        self._write_collect_now(f"{self.install_id.upper()} 42\n")
        self.assertEqual(receiver._collect_now_epoch(self.install_id), 42)
        self.assertEqual(receiver._collect_now_epoch("other-install"), 0)

    def test_diag_installs_ignores_blank_and_malformed_lines(self) -> None:
        self._write_diag_installs(
            f"\n  {self.install_id}  \nnot valid! id\nabcdef01-2345\n"
        )
        armed = receiver._armed_installs()
        self.assertIn(self.install_id, armed)
        self.assertIn("abcdef01-2345", armed)
        self.assertNotIn("not valid! id", armed)

    def test_diag_installs_absent_file_is_empty(self) -> None:
        self.assertEqual(receiver._armed_installs(), set())

    def test_armed_installs_reread_every_call_no_caching(self) -> None:
        self.assertEqual(receiver._armed_installs(), set())
        self._write_diag_installs(f"{self.install_id}\n")
        self.assertIn(self.install_id, receiver._armed_installs())
        # Disarm takes effect immediately on the next read — no restart.
        self._write_diag_installs("")
        self.assertEqual(receiver._armed_installs(), set())


class HangDiagnosticsHttpTests(unittest.TestCase):
    install_id = "12345678-abcd"

    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.original_root = receiver.ROOT
        self.addCleanup(setattr, receiver, "ROOT", self.original_root)
        receiver.ROOT = self.directory.name

        with mock.patch("socket.getfqdn", return_value="localhost"):
            self.server = receiver.ThreadingHTTPServer(
                ("127.0.0.1", 0), receiver.Handler
            )
        self.server.daemon_threads = True
        self.addCleanup(self.server.server_close)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.thread.join, 2)
        self.addCleanup(self.server.shutdown)

    def _arm(self, install_id: str | None = None) -> None:
        (Path(receiver.ROOT) / receiver.DIAG_INSTALLS_FILENAME).write_text(
            (install_id or self.install_id) + "\n", encoding="utf-8"
        )

    def _set_collect_now(self, install_id: str, epoch: int) -> None:
        (Path(receiver.ROOT) / receiver.COLLECT_NOW_FILENAME).write_text(
            f"{install_id} {epoch}\n", encoding="utf-8"
        )

    def post(
        self, path: str, body: bytes, headers: dict[str, str]
    ) -> tuple[int, dict[str, str], bytes]:
        connection = http.client.HTTPConnection(
            "127.0.0.1", self.server.server_address[1], timeout=5
        )
        connection.request("POST", path, body=body, headers=headers)
        response = connection.getresponse()
        payload = response.read()
        response_headers = dict(response.getheaders())
        status = response.status
        connection.close()
        return status, response_headers, payload

    def _ingest_headers(self) -> dict[str, str]:
        return {
            "Authorization": "Bearer " + receiver.TOKEN,
            "X-Install-Id": self.install_id,
        }

    def _ingest_payload(self) -> bytes:
        item = event("audio readiness accepted", timestamp="2026-08-19T00:00:00Z")
        return (json.dumps(item) + "\n").encode("utf-8")

    # -- /ingest reply-shape arming -----------------------------------

    def test_unarmed_install_gets_bare_204(self) -> None:
        status, headers, body = self.post(
            "/ingest", self._ingest_payload(), self._ingest_headers()
        )
        self.assertEqual(status, 204)
        self.assertEqual(body, b"")

    def test_armed_install_gets_200_json_diagnostics_true(self) -> None:
        self._arm()
        status, headers, body = self.post(
            "/ingest", self._ingest_payload(), self._ingest_headers()
        )
        self.assertEqual(status, 200)
        self.assertEqual(headers.get("Content-Type"), "application/json")
        payload = json.loads(body)
        self.assertIs(payload["diagnostics"], True)
        self.assertEqual(payload.get("collect_now", 0), 0)

    def test_armed_install_reply_carries_collect_now_epoch(self) -> None:
        self._arm()
        self._set_collect_now(self.install_id, 1785873094)
        status, _, body = self.post(
            "/ingest", self._ingest_payload(), self._ingest_headers()
        )
        payload = json.loads(body)
        self.assertEqual(status, 200)
        self.assertEqual(payload["collect_now"], 1785873094)

    def test_disarming_takes_effect_on_the_very_next_ingest(self) -> None:
        self._arm()
        armed_status, _, _ = self.post(
            "/ingest", self._ingest_payload(), self._ingest_headers()
        )
        (Path(receiver.ROOT) / receiver.DIAG_INSTALLS_FILENAME).unlink()
        disarmed_status, _, disarmed_body = self.post(
            "/ingest", self._ingest_payload(), self._ingest_headers()
        )
        self.assertEqual(armed_status, 200)
        self.assertEqual(disarmed_status, 204)
        self.assertEqual(disarmed_body, b"")

    # -- POST /bundle ----------------------------------------------------

    def test_bundle_requires_auth(self) -> None:
        self._arm()
        status, _, _ = self.post(
            "/bundle",
            b"bundle text",
            {"Authorization": "Bearer wrong", "X-Install-Id": self.install_id},
        )
        self.assertEqual(status, 401)

    def test_bundle_rejects_unarmed_install(self) -> None:
        status, _, body = self.post(
            "/bundle",
            b"bundle text",
            {
                "Authorization": "Bearer " + receiver.TOKEN,
                "X-Install-Id": self.install_id,
            },
        )
        self.assertEqual(status, 403)
        self.assertEqual(body, b"install not armed")
        self.assertFalse(
            list((Path(receiver.ROOT) / self.install_id).glob("hang-bundle-*.txt"))
            if (Path(receiver.ROOT) / self.install_id).exists()
            else []
        )

    def test_bundle_rejects_oversize_request(self) -> None:
        self._arm()
        with mock.patch.object(receiver, "MAX_BODY", 16):
            status, _, _ = self.post(
                "/bundle",
                b"x" * 32,
                {
                    "Authorization": "Bearer " + receiver.TOKEN,
                    "X-Install-Id": self.install_id,
                    "Content-Length": "32",
                },
            )
        self.assertEqual(status, 413)

    def test_bundle_success_writes_exact_bytes_and_replies_success(self) -> None:
        self._arm()
        body = b"===== hang context =====\nworker native stack sample\n"
        status, _, reply_body = self.post(
            "/bundle",
            body,
            {
                "Authorization": "Bearer " + receiver.TOKEN,
                "X-Install-Id": self.install_id,
            },
        )
        self.assertTrue(200 <= status < 300, f"expected 2xx, got {status}")
        self.assertEqual(reply_body, b"")

        install_dir = Path(receiver.ROOT) / self.install_id
        bundles = list(install_dir.glob("hang-bundle-*.txt"))
        self.assertEqual(len(bundles), 1)
        self.assertEqual(bundles[0].read_bytes(), body)
        self.assertRegex(bundles[0].name, r"^hang-bundle-\d+\.txt$")
        self.assertFalse(list(install_dir.glob("*.tmp")))

    def test_bundle_never_touches_the_event_store(self) -> None:
        self._arm()
        self.post(
            "/bundle",
            b"opaque diagnostic text",
            {
                "Authorization": "Bearer " + receiver.TOKEN,
                "X-Install-Id": self.install_id,
            },
        )
        self.assertEqual(receiver.event_store().event_count(self.install_id), 0)

    def test_bundle_enforces_per_install_quota(self) -> None:
        self._arm()
        install_dir = Path(receiver.ROOT) / self.install_id
        install_dir.mkdir(parents=True)
        (install_dir / "events.jsonl").write_bytes(b"x" * 90)

        with mock.patch.object(receiver, "MAX_FILE", 100):
            under_status, _, _ = self.post(
                "/bundle",
                b"y" * 5,
                {
                    "Authorization": "Bearer " + receiver.TOKEN,
                    "X-Install-Id": self.install_id,
                },
            )
            over_status, _, over_body = self.post(
                "/bundle",
                b"z" * 50,
                {
                    "Authorization": "Bearer " + receiver.TOKEN,
                    "X-Install-Id": self.install_id,
                },
            )

        self.assertTrue(200 <= under_status < 300, under_status)
        self.assertEqual(over_status, 507)
        self.assertEqual(over_body, b"storage unavailable")


if __name__ == "__main__":
    unittest.main()
