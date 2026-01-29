#!/usr/bin/env python3
"""
Local Voice Dictation Tool
A privacy-first voice-to-text tool inspired by Wispr Flow.

Usage:
    python main.py [--no-cleanup] [--hotkey alt_r] [--model llama3.2:3b]
"""

import argparse
import sys
import time
import json
from pathlib import Path
from datetime import datetime

import psutil

from audio_recorder import AudioRecorder
from llm_cleanup import LLMCleanup
from text_injector import TextInjector
from hotkey_listener import HotkeyListener


class VoiceDictation:
    """Main voice dictation application."""

    def __init__(
        self,
        hotkey: str = "alt_r",
        use_cleanup: bool = True,
        llm_model: str = "llama3.2:3b",
        whisper_model: str = "base.en",
        injection_method: str = "paste",
        backend: str = "openai"
    ):
        """
        Initialize the voice dictation tool.

        Args:
            hotkey: Hotkey to trigger recording
            use_cleanup: Whether to use LLM cleanup
            llm_model: Ollama model for cleanup
            whisper_model: Whisper model name (tiny.en, base.en, small.en, turbo)
            injection_method: "paste" or "type"
            backend: "openai" for OpenAI Whisper, "cpp" for whisper.cpp
        """
        print("Initializing Voice Dictation...")
        print(f"Backend: {backend}")

        # Initialize components
        self.recorder = AudioRecorder()

        # Load the appropriate transcriber based on backend
        if backend == "cpp":
            from transcriber_cpp import WhisperCppTranscriber
            self.transcriber = WhisperCppTranscriber(model_name=whisper_model)
        elif backend == "deepgram":
            from transcriber_deepgram import DeepgramTranscriber
            self.transcriber = DeepgramTranscriber(model_name=whisper_model)
        else:
            from transcriber import WhisperTranscriber
            self.transcriber = WhisperTranscriber(model_name=whisper_model)

        self.cleanup = LLMCleanup(model=llm_model, enabled=use_cleanup)
        self.injector = TextInjector(method=injection_method)

        # Initialize hotkey listener
        self.listener = HotkeyListener(
            on_press=self._on_hotkey_press,
            on_release=self._on_hotkey_release,
            hotkey=hotkey
        )

        self._processing = False

        # Setup logging
        self.log_file = Path("dictation_log.jsonl")
        self.session_start = datetime.now().isoformat()

        print(f"Hotkey: {hotkey}")
        print(f"LLM Cleanup: {'Enabled' if use_cleanup else 'Disabled'}")
        print(f"Log file: {self.log_file.absolute()}")
        if use_cleanup:
            if self.cleanup.is_available():
                print(f"Ollama: Connected ({llm_model})")
            else:
                print("Ollama: Not available (will use raw transcription)")

    def _on_hotkey_press(self):
        """Called when hotkey is pressed."""
        if self._processing:
            return
        print("[RECORDING...] Speak now (release shift to stop)")
        self.recorder.start_recording()

    def _on_hotkey_release(self):
        """Called when hotkey is released."""
        if not self.recorder.is_recording:
            return

        self._processing = True
        start_time = time.time()

        try:
            process = psutil.Process()

            # Stop recording and get audio file
            t0 = time.time()
            audio_file = self.recorder.stop_recording()
            record_time = time.time() - t0

            if audio_file is None:
                print("No audio recorded")
                return

            # Audio file size
            try:
                audio_size = audio_file.stat().st_size / 1024  # KB
            except:
                audio_size = 0

            # --- WHISPER TRANSCRIPTION ---
            mem_before_whisper = process.memory_info().rss / 1024 / 1024
            cpu_before_whisper = psutil.cpu_percent(interval=None)

            t0 = time.time()
            text = self.transcriber.transcribe(audio_file)
            transcribe_time = time.time() - t0

            mem_after_whisper = process.memory_info().rss / 1024 / 1024
            cpu_after_whisper = psutil.cpu_percent(interval=0.1)
            whisper_mem_delta = mem_after_whisper - mem_before_whisper

            if not text.strip():
                print("No speech detected")
                return

            # --- OLLAMA CLEANUP ---
            if self.cleanup.enabled:
                mem_before_ollama = process.memory_info().rss / 1024 / 1024

                t0 = time.time()
                final_text = self.cleanup.cleanup(text)
                cleanup_time = time.time() - t0

                mem_after_ollama = process.memory_info().rss / 1024 / 1024
                cpu_after_ollama = psutil.cpu_percent(interval=0.1)
                ollama_mem_delta = mem_after_ollama - mem_before_ollama
            else:
                final_text = text
                cleanup_time = 0
                cpu_after_ollama = 0
                ollama_mem_delta = 0

            # --- TOTALS ---
            total_time = time.time() - start_time

            # Check GPU
            try:
                import torch
                if torch.backends.mps.is_available():
                    gpu_status = "MPS (Apple GPU)"
                elif torch.cuda.is_available():
                    gpu_status = "CUDA"
                else:
                    gpu_status = "CPU only"
            except:
                gpu_status = "N/A"

            # Print text
            print(f"\n[RAW]     {text}")
            print(f"[CLEANED] {final_text}")

            # Print all stats together
            print(f"\n[STATS]")
            print(f"  WHISPER: {transcribe_time:.2f}s | CPU: {cpu_after_whisper:.1f}% | RAM: {mem_after_whisper:.0f}MB ({whisper_mem_delta:+.1f}MB)")
            if self.cleanup.enabled:
                print(f"  OLLAMA:  {cleanup_time:.2f}s | CPU: {cpu_after_ollama:.1f}% | RAM: {mem_after_ollama:.0f}MB ({ollama_mem_delta:+.1f}MB)")
            else:
                print(f"  OLLAMA:  Disabled (--no-cleanup)")
            print(f"  TOTAL:   {total_time:.2f}s | Audio: {audio_size:.1f}KB | GPU: {gpu_status}\n")

            # Log to file
            log_entry = {
                "timestamp": datetime.now().isoformat(),
                "session": self.session_start,
                "raw_text": text,
                "cleaned_text": final_text,
                "audio_size_kb": round(audio_size, 1),
                "whisper_time_s": round(transcribe_time, 2),
                "whisper_cpu_pct": round(cpu_after_whisper, 1),
                "whisper_ram_mb": round(mem_after_whisper, 0),
                "ollama_enabled": self.cleanup.enabled,
                "ollama_time_s": round(cleanup_time, 2) if self.cleanup.enabled else None,
                "total_time_s": round(total_time, 2),
                "gpu": gpu_status
            }
            with open(self.log_file, "a") as f:
                f.write(json.dumps(log_entry) + "\n")

            # Inject into focused app
            self.injector.inject(final_text)

            # Cleanup temp file
            try:
                audio_file.unlink()
            except:
                pass

        finally:
            self._processing = False

    def run(self):
        """Start the voice dictation tool."""
        print("\n" + "=" * 50)
        print("Voice Dictation Ready!")
        print("=" * 50)
        print(f"\nHold the hotkey to record, release to transcribe and paste.")
        print("Press Ctrl+C to exit.\n")

        try:
            self.listener.start()
            # Use a loop instead of join() so Ctrl+C works
            while True:
                time.sleep(0.1)
        except KeyboardInterrupt:
            self.shutdown()

    def shutdown(self):
        """Clean shutdown."""
        print("\n\nShutting down...")
        self.listener.stop()
        if self.recorder.is_recording:
            self.recorder.stop_recording()
        print("Goodbye!")
        import sys
        sys.exit(0)


def main():
    parser = argparse.ArgumentParser(
        description="Local Voice Dictation Tool"
    )
    parser.add_argument(
        "--hotkey",
        default="shift_l",
        help="Hotkey to trigger recording (default: shift_l)"
    )
    parser.add_argument(
        "--cleanup",
        action="store_true",
        help="Enable LLM cleanup (disabled by default)"
    )
    parser.add_argument(
        "--model",
        default="llama3.2:3b",
        help="Ollama model for cleanup (default: llama3.2:3b)"
    )
    parser.add_argument(
        "--whisper-model",
        default="turbo",
        help="Whisper model name: tiny.en, base.en, small.en, turbo (default: turbo)"
    )
    parser.add_argument(
        "--type",
        action="store_true",
        help="Use typing instead of clipboard paste"
    )
    parser.add_argument(
        "--backend",
        default="openai",
        choices=["openai", "cpp", "deepgram"],
        help="Transcription backend: openai, cpp, or deepgram"
    )
    parser.add_argument(
        "--cpp",
        action="store_true",
        help="Shorthand for --backend cpp"
    )
    parser.add_argument(
        "--deepgram",
        action="store_true",
        help="Shorthand for --backend deepgram (requires DEEPGRAM_API_KEY env var)"
    )

    args = parser.parse_args()

    # Handle shorthand flags
    if args.deepgram:
        backend = "deepgram"
    elif args.cpp:
        backend = "cpp"
    else:
        backend = args.backend

    app = VoiceDictation(
        hotkey=args.hotkey,
        use_cleanup=args.cleanup,
        llm_model=args.model,
        whisper_model=args.whisper_model,
        injection_method="type" if args.type else "paste",
        backend=backend
    )

    app.run()


if __name__ == "__main__":
    main()
