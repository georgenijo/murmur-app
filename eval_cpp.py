#!/usr/bin/env python3
"""
Evaluate whisper.cpp with different model sizes.

Usage:
    python eval_cpp.py <audio_file>
"""

import sys
import time
from pathlib import Path

from transcriber_cpp import WhisperCppTranscriber


def test_model(model_name, audio_path):
    """Test a single model and return results."""
    print(f"\n{'='*60}")
    print(f"Testing whisper.cpp: {model_name}")
    print('='*60)

    try:
        # Initialize
        t0 = time.time()
        transcriber = WhisperCppTranscriber(model_name=model_name)
        init_time = time.time() - t0

        # Transcribe
        t0 = time.time()
        result = transcriber.transcribe(audio_path)
        transcribe_time = time.time() - t0

        print(f"Result: {result[:100]}..." if len(result) > 100 else f"Result: {result}")

        return {
            "model": model_name,
            "init_time": init_time,
            "transcribe_time": transcribe_time,
            "result": result,
            "error": None
        }
    except Exception as e:
        print(f"Error: {e}")
        return {
            "model": model_name,
            "init_time": 0,
            "transcribe_time": 0,
            "result": "",
            "error": str(e)
        }


def main():
    if len(sys.argv) < 2:
        print("Usage: python eval_cpp.py <audio_file>")
        print()
        print("Example:")
        print("  python record_test.py 15")
        print("  python eval_cpp.py test_audio.wav")
        sys.exit(1)

    audio_path = sys.argv[1]

    if not Path(audio_path).exists():
        print(f"Error: Audio file not found: {audio_path}")
        sys.exit(1)

    print(f"Audio file: {audio_path}")
    print(f"File size: {Path(audio_path).stat().st_size / 1024:.1f} KB")

    # Models to test (small to large)
    models = [
        "tiny.en",
        "base.en",
        "small.en",
    ]

    results = []
    for model in models:
        results.append(test_model(model, audio_path))

    # Print comparison
    print("\n")
    print("="*70)
    print("WHISPER.CPP MODEL COMPARISON")
    print("="*70)

    print(f"\n{'Model':<12} {'Init':<10} {'Transcribe':<12} {'Total':<10} {'Status'}")
    print("-"*70)

    for r in results:
        if r["error"]:
            print(f"{r['model']:<12} ERROR: {r['error']}")
        else:
            total = r["init_time"] + r["transcribe_time"]
            print(f"{r['model']:<12} {r['init_time']:.2f}s      {r['transcribe_time']:.2f}s        {total:.2f}s      OK")

    print("\n" + "-"*70)
    print("TRANSCRIPTION OUTPUTS:")
    print("-"*70)

    for r in results:
        if not r["error"] and r["result"]:
            print(f"\n[{r['model']}]")
            print(f"  {r['result']}")

    # Find fastest
    valid_results = [r for r in results if not r["error"] and r["result"]]
    if valid_results:
        fastest = min(valid_results, key=lambda x: x["transcribe_time"])
        print(f"\n{'='*70}")
        print(f"FASTEST: {fastest['model']} ({fastest['transcribe_time']:.2f}s)")
        print(f"{'='*70}")


if __name__ == "__main__":
    main()
