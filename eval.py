#!/usr/bin/env python3
"""
Evaluation script to compare transcription backends.

Usage:
    python eval.py <audio_file>

This will run the same audio through all available backends
and compare speed and output.
"""

import sys
import time
from pathlib import Path

# Add project to path
sys.path.insert(0, str(Path(__file__).parent))

from dotenv import load_dotenv
load_dotenv(Path(__file__).parent / ".env")


def test_backend(name, transcriber_class, model_name, audio_path):
    """Test a single backend and return results."""
    print(f"\n{'='*60}")
    print(f"Testing: {name} (model: {model_name})")
    print('='*60)

    try:
        # Initialize
        t0 = time.time()
        transcriber = transcriber_class(model_name=model_name)
        init_time = time.time() - t0

        # Transcribe
        t0 = time.time()
        result = transcriber.transcribe(audio_path)
        transcribe_time = time.time() - t0

        return {
            "name": name,
            "model": model_name,
            "init_time": init_time,
            "transcribe_time": transcribe_time,
            "result": result,
            "error": None
        }
    except Exception as e:
        return {
            "name": name,
            "model": model_name,
            "init_time": 0,
            "transcribe_time": 0,
            "result": "",
            "error": str(e)
        }


def main():
    if len(sys.argv) < 2:
        print("Usage: python eval.py <audio_file>")
        print()
        print("To create a test audio file, run:")
        print("  python main.py")
        print("  (record something, then find the temp file in logs)")
        print()
        print("Or record directly:")
        print("  python audio_recorder.py")
        sys.exit(1)

    audio_path = sys.argv[1]

    if not Path(audio_path).exists():
        print(f"Error: Audio file not found: {audio_path}")
        sys.exit(1)

    print(f"Audio file: {audio_path}")
    print(f"File size: {Path(audio_path).stat().st_size / 1024:.1f} KB")

    results = []

    # Test OpenAI Whisper (base.en - fast)
    try:
        from transcriber import WhisperTranscriber
        results.append(test_backend(
            "OpenAI Whisper",
            WhisperTranscriber,
            "base.en",
            audio_path
        ))
    except ImportError as e:
        print(f"Skipping OpenAI Whisper: {e}")

    # Test whisper.cpp (base.en - fast)
    try:
        from transcriber_cpp import WhisperCppTranscriber
        results.append(test_backend(
            "whisper.cpp",
            WhisperCppTranscriber,
            "base.en",
            audio_path
        ))
    except ImportError as e:
        print(f"Skipping whisper.cpp: {e}")

    # Test Deepgram
    try:
        from transcriber_deepgram import DeepgramTranscriber
        results.append(test_backend(
            "Deepgram API",
            DeepgramTranscriber,
            "nova-2",
            audio_path
        ))
    except ImportError as e:
        print(f"Skipping Deepgram: {e}")

    # Print comparison
    print("\n")
    print("="*60)
    print("COMPARISON RESULTS")
    print("="*60)

    print(f"\n{'Backend':<20} {'Model':<12} {'Init':<8} {'Transcribe':<12} {'Total':<10}")
    print("-"*70)

    for r in results:
        if r["error"]:
            print(f"{r['name']:<20} {r['model']:<12} ERROR: {r['error']}")
        else:
            total = r["init_time"] + r["transcribe_time"]
            print(f"{r['name']:<20} {r['model']:<12} {r['init_time']:.2f}s    {r['transcribe_time']:.2f}s        {total:.2f}s")

    print("\n" + "-"*70)
    print("TRANSCRIPTION OUTPUTS:")
    print("-"*70)

    for r in results:
        if not r["error"]:
            print(f"\n[{r['name']}]")
            print(f"  {r['result'][:200]}{'...' if len(r['result']) > 200 else ''}")

    # Find fastest
    valid_results = [r for r in results if not r["error"]]
    if valid_results:
        fastest = min(valid_results, key=lambda x: x["transcribe_time"])
        print(f"\n{'='*60}")
        print(f"FASTEST: {fastest['name']} ({fastest['transcribe_time']:.2f}s)")
        print(f"{'='*60}")


if __name__ == "__main__":
    main()
