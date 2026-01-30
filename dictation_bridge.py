#!/usr/bin/env python3
"""
Dictation Bridge - JSON-based interface for Tauri sidecar communication.
Reads JSON commands from stdin, writes JSON responses to stdout.
"""

import sys
import json
import os
from enum import Enum
from pathlib import Path

# Debug logging to file
DEBUG_LOG = "/tmp/dictation_bridge_debug.log"

def debug_log(message):
    with open(DEBUG_LOG, "a") as f:
        f.write(f"{message}\n")
        f.flush()

# Ensure we can import from the same directory
sys.path.insert(0, str(Path(__file__).parent))

# Import existing modules
from audio_recorder import AudioRecorder
from transcriber_cpp import WhisperCppTranscriber
from text_injector import TextInjector


class State(Enum):
    IDLE = "idle"
    RECORDING = "recording"
    PROCESSING = "processing"


class DictationBridge:
    """Bridge class that handles JSON commands for dictation functionality."""

    def __init__(self):
        """Initialize the DictationBridge with default settings."""
        self.state = State.IDLE
        self.backend = "cpp"
        self.model = "large-v3-turbo"
        self.language = "en"
        self.recorder = None
        self.transcriber = None
        self.injector = TextInjector(method="paste")
        self._init_components()

    def _init_components(self):
        """Initialize audio recorder. Transcriber is initialized lazily."""
        import io
        old_stdout = sys.stdout
        old_stderr = sys.stderr
        sys.stdout = io.StringIO()
        sys.stderr = io.StringIO()
        try:
            self.recorder = AudioRecorder()
        finally:
            sys.stdout = old_stdout
            sys.stderr = old_stderr
        self.transcriber = None  # Lazy initialization

    def _get_transcriber(self):
        """Get the transcriber, initializing it lazily if needed.

        This method suppresses stdout/stderr during model loading to prevent
        whisper.cpp loading messages from breaking the JSON protocol.

        Returns:
            WhisperCppTranscriber instance.
        """
        if self.transcriber is None:
            # Redirect stdout/stderr during model loading to suppress
            # whisper.cpp "Loading whisper.cpp model..." messages
            import io
            old_stdout = sys.stdout
            old_stderr = sys.stderr
            sys.stdout = io.StringIO()
            sys.stderr = io.StringIO()
            try:
                self.transcriber = WhisperCppTranscriber(model_name=self.model)
            finally:
                sys.stdout = old_stdout
                sys.stderr = old_stderr
        return self.transcriber

    def handle_command(self, cmd_data):
        """Handle an incoming command and return a response.

        Args:
            cmd_data: Dictionary containing the command and any parameters.

        Returns:
            Dictionary containing the response.
        """
        cmd = cmd_data.get("cmd")

        if cmd == "get_status":
            return self._get_status()
        elif cmd == "start_recording":
            return self._start_recording()
        elif cmd == "stop_recording":
            return self._stop_recording()
        elif cmd == "configure":
            return self._configure(cmd_data)
        elif cmd == "shutdown":
            return {"type": "ack", "cmd": "shutdown"}
        else:
            return {"type": "error", "message": f"Unknown command: {cmd}", "code": "UNKNOWN_CMD"}

    def _get_status(self):
        """Return the current status of the bridge.

        Returns:
            Dictionary with type, state, model, and backend information.
        """
        return {
            "type": "status",
            "state": self.state.value,
            "model": self.model,
            "backend": self.backend,
            "language": self.language
        }

    def _start_recording(self):
        """Start audio recording.

        Returns:
            Dictionary with acknowledgment or error.
        """
        if self.state == State.RECORDING:
            return {"type": "error", "message": "Already recording", "code": "ALREADY_RECORDING"}

        try:
            self.recorder.start_recording()
            self.state = State.RECORDING
            return {"type": "ack", "cmd": "start_recording", "state": self.state.value}
        except Exception as e:
            error_msg = str(e)
            # Check for common permission errors
            if "permission" in error_msg.lower() or "denied" in error_msg.lower():
                return {"type": "error", "message": "Microphone access denied. Please grant permission in System Settings.", "code": "MIC_PERMISSION_DENIED"}
            return {"type": "error", "message": f"Recording failed: {error_msg}", "code": "RECORDING_FAILED"}

    def _stop_recording(self):
        """Stop recording and transcribe the audio.

        Returns:
            Dictionary with transcription result or error.
        """
        if self.state != State.RECORDING:
            return {"type": "error", "message": "Not recording", "code": "NOT_RECORDING"}

        try:
            self.state = State.PROCESSING
            debug_log("Stopping recording...")

            audio_file = self.recorder.stop_recording()

            if audio_file is None or not audio_file.exists():
                debug_log("ERROR: No audio file returned")
                self.state = State.IDLE
                return {"type": "error", "message": "No audio recorded", "code": "NO_AUDIO"}

            # Log audio file details
            file_size = audio_file.stat().st_size
            debug_log(f"Audio file: {audio_file}, size: {file_size} bytes")

            # Get audio duration (rough estimate: 16kHz mono 16-bit = 32000 bytes/sec)
            duration_estimate = file_size / 32000
            debug_log(f"Estimated audio duration: {duration_estimate:.2f} seconds")

            if file_size < 5000:  # Less than ~0.15 seconds
                debug_log("WARNING: Very short recording, might just be noise")

            # Check audio levels to detect silent recordings
            import numpy as np
            from scipy.io import wavfile

            try:
                sample_rate, audio_data = wavfile.read(audio_file)
                max_level = np.max(np.abs(audio_data))
                mean_level = np.mean(np.abs(audio_data))
                debug_log(f"Audio levels - max: {max_level}, mean: {mean_level:.2f}")

                if max_level < 100:
                    debug_log("WARNING: Audio is SILENT - microphone not capturing! Check permissions.")
            except Exception as e:
                debug_log(f"Could not analyze audio levels: {e}")

            # Check if the audio file has content
            if file_size == 0:
                self.state = State.IDLE
                # Clean up empty file
                try:
                    audio_file.unlink()
                except OSError:
                    pass
                return {"type": "error", "message": "No audio recorded", "code": "NO_AUDIO"}

            # Transcribe the audio
            import time
            start_time = time.time()
            debug_log("Starting transcription...")

            text = self._get_transcriber().transcribe(str(audio_file), language=self.language)

            transcription_time = time.time() - start_time
            debug_log(f"Transcription completed in {transcription_time:.2f}s: '{text}'")

            # Save a copy for debugging instead of deleting
            import shutil
            debug_audio_path = "/tmp/dictation_last_recording.wav"
            try:
                shutil.copy(audio_file, debug_audio_path)
                debug_log(f"Saved debug copy to {debug_audio_path}")
                os.unlink(audio_file)
            except Exception as e:
                debug_log(f"Failed to save debug copy: {e}")

            self.state = State.IDLE

            # Auto-paste the transcription
            if text and text.strip():
                try:
                    debug_log(f"Auto-pasting text: '{text[:50]}...'")
                    self.injector.inject(text)
                    debug_log("Text pasted successfully")
                except Exception as e:
                    debug_log(f"Failed to auto-paste: {e}")

            return {
                "type": "transcription",
                "text": text.strip() if text else "",
                "raw_text": text if text else "",
                "duration": duration_estimate,
                "transcription_time": transcription_time
            }
        except RuntimeError as e:
            debug_log(f"ERROR in stop_recording: {e}")
            self.state = State.IDLE
            return {"type": "error", "message": str(e), "code": "TRANSCRIPTION_FAILED"}
        except Exception as e:
            debug_log(f"ERROR in stop_recording: {e}")
            self.state = State.IDLE
            return {"type": "error", "message": str(e), "code": "TRANSCRIPTION_FAILED"}

    def _configure(self, cmd_data):
        """Configure the bridge settings.

        Args:
            cmd_data: Dictionary containing configuration options.

        Returns:
            Dictionary with acknowledgment of the configuration.
        """
        reconfigure_transcriber = False

        if "model" in cmd_data and cmd_data["model"] != self.model:
            self.model = cmd_data["model"]
            reconfigure_transcriber = True

        if "backend" in cmd_data:
            self.backend = cmd_data["backend"]

        if "language" in cmd_data:
            self.language = cmd_data["language"]

        # Reset transcriber so it reloads with new model on next use
        if reconfigure_transcriber:
            self.transcriber = None

        return {
            "type": "ack",
            "cmd": "configure",
            "model": self.model,
            "backend": self.backend,
            "language": self.language
        }


def main():
    """Main entry point for the dictation bridge."""
    bridge = DictationBridge()

    # Send ready status
    print(json.dumps({"type": "ready", "state": "idle"}), flush=True)
    debug_log("Sent ready message")

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        debug_log(f"Received: {line}")

        try:
            cmd_data = json.loads(line)
            response = bridge.handle_command(cmd_data)
            debug_log(f"Sending: {response}")
            print(json.dumps(response), flush=True)

            if cmd_data.get("cmd") == "shutdown":
                break
        except json.JSONDecodeError as e:
            print(json.dumps({
                "type": "error",
                "message": f"Invalid JSON: {e}",
                "code": "INVALID_JSON"
            }), flush=True)
        except Exception as e:
            print(json.dumps({
                "type": "error",
                "message": str(e),
                "code": "INTERNAL_ERROR"
            }), flush=True)


if __name__ == "__main__":
    main()
