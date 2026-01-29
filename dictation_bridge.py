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

# Ensure we can import from the same directory
sys.path.insert(0, str(Path(__file__).parent))

# Import existing modules
from audio_recorder import AudioRecorder
from transcriber_cpp import WhisperCppTranscriber


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
        self._init_components()

    def _init_components(self):
        """Initialize the audio recorder and transcriber components."""
        self.recorder = AudioRecorder()
        self.transcriber = WhisperCppTranscriber(model_name=self.model)

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
        except RuntimeError as e:
            return {"type": "error", "message": str(e), "code": "RECORDING_FAILED"}
        except Exception as e:
            return {"type": "error", "message": str(e), "code": "RECORDING_FAILED"}

    def _stop_recording(self):
        """Stop recording and transcribe the audio.

        Returns:
            Dictionary with transcription result or error.
        """
        if self.state != State.RECORDING:
            return {"type": "error", "message": "Not recording", "code": "NOT_RECORDING"}

        try:
            self.state = State.PROCESSING
            audio_file = self.recorder.stop_recording()

            if audio_file is None or not audio_file.exists():
                self.state = State.IDLE
                return {"type": "error", "message": "No audio recorded", "code": "NO_AUDIO"}

            # Check if the audio file has content
            if audio_file.stat().st_size == 0:
                self.state = State.IDLE
                # Clean up empty file
                try:
                    audio_file.unlink()
                except OSError:
                    pass
                return {"type": "error", "message": "No audio recorded", "code": "NO_AUDIO"}

            # Transcribe the audio
            text = self.transcriber.transcribe(str(audio_file), language=self.language)

            # Clean up the temporary audio file
            try:
                audio_file.unlink()
            except OSError:
                pass

            self.state = State.IDLE

            return {
                "type": "transcription",
                "text": text.strip() if text else "",
                "raw_text": text if text else ""
            }
        except RuntimeError as e:
            self.state = State.IDLE
            return {"type": "error", "message": str(e), "code": "TRANSCRIPTION_FAILED"}
        except Exception as e:
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

        # Reinitialize transcriber if model changed
        if reconfigure_transcriber:
            self.transcriber = WhisperCppTranscriber(model_name=self.model)

        return {
            "type": "ack",
            "cmd": "configure",
            "model": self.model,
            "backend": self.backend,
            "language": self.language
        }


def main():
    """Main entry point for the dictation bridge."""
    # Redirect stderr to suppress whisper.cpp model loading messages
    # This prevents them from being mixed with JSON output
    stderr_backup = sys.stderr
    sys.stderr = open(os.devnull, 'w')

    try:
        bridge = DictationBridge()
    finally:
        # Restore stderr
        sys.stderr.close()
        sys.stderr = stderr_backup

    # Send ready status
    print(json.dumps({"type": "ready", "state": "idle"}), flush=True)

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            cmd_data = json.loads(line)
            response = bridge.handle_command(cmd_data)
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
