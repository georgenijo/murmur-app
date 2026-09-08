# Capture first-PCM smoke

`scripts/smoke_test_capture_first_pcm.py` checks both native microphone backends against a protocol-v9 capture worker. It detects an AUHAL regression even when CPAL still works.

The runner needs an open-lid Apple Silicon Mac with a connected built-in microphone and microphone permission for the process responsible for the worker. The probe pins the built-in default input by its UID. If the default is not built-in, exactly one connected built-in input must exist. Device names and UIDs stay in process memory.

```bash
python3 scripts/smoke_test_capture_first_pcm.py --worker /path/to/murmur-capture-worker --inventory-only
python3 scripts/smoke_test_capture_first_pcm.py --worker /path/to/murmur-capture-worker
```

`--inventory-only` checks protocol negotiation and input selection without opening a capture stream. The full command opens AUHAL and CPAL separately, with no fallback between attempts. Each backend must satisfy all these conditions:

- Every expected setup step enters and completes in order, including the individual AUHAL native calls.
- The pinned input resolves successfully. `streamOpen` precedes setup, `awaitingFirstCallback` occurs between that setup step's entry and completion, and `active` follows all completed setup steps.
- At least three microphone PCM frames arrive with continuous sequence numbers and sample offsets, a stable positive sample rate, and nondecreasing timestamps.
- The first PCM frame arrives less than two seconds after the start command.
- The worker acknowledges stop within two seconds, reports the exact received sample count, and exits successfully within the same stop deadline.

`--first-pcm-seconds` changes the startup threshold. `--timeout-seconds` sets the complete session deadline, including protocol negotiation. Both options require a finite value greater than zero and at most 30 seconds. Their defaults are two and ten seconds respectively.

Each worker starts in a new process group. Failure, interruption, or expiry triggers a hard kill of that owned group and a bounded wait for its leader. The probe never discovers or terminates another Murmur process. PCM payloads are discarded after framing validation. The JSON result contains backend names, counts, durations, and fixed error descriptions. The probe does not save or upload audio.

The existing `scripts/smoke_test_capture_worker.py` remains the lightweight signed-release protocol check. It stops at `streamOpen` and does not prove first PCM. Hardware-free tests in `tests/test_capture_first_pcm.py` validate the deeper probe's parser and failure handling. Those tests do not establish physical microphone behavior.
