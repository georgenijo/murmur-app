#!/usr/bin/env python3
"""
Test all whisper.cpp model weights.

Usage:
    python tests/test_cpp.py <audio_file> --reference "the actual words spoken"
    python tests/test_cpp.py test_audio.wav --reference "hello world this is a test"
"""

import argparse
import sys
import time
from pathlib import Path

# Add project root to path
sys.path.insert(0, str(Path(__file__).parent.parent))
sys.path.insert(0, str(Path(__file__).parent))

from transcriber_cpp import WhisperCppTranscriber
from accuracy import calculate_accuracy, format_accuracy

# All available whisper.cpp models (English-only versions for speed)
MODELS = [
    "tiny.en",
    "base.en",
    "small.en",
    "medium.en",
    "large-v3",
    "large-v3-turbo",
]


def test_model(model_name: str, audio_path: str, reference: str = None) -> dict:
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

        # Calculate accuracy if reference provided
        accuracy_metrics = None
        if reference:
            accuracy_metrics = calculate_accuracy(reference, result)
            print(f"Accuracy: {format_accuracy(accuracy_metrics)}")

        return {
            "model": model_name,
            "init_time": init_time,
            "transcribe_time": transcribe_time,
            "total_time": init_time + transcribe_time,
            "result": result,
            "accuracy": accuracy_metrics,
            "error": None
        }
    except Exception as e:
        print(f"Error: {e}")
        return {
            "model": model_name,
            "init_time": 0,
            "transcribe_time": 0,
            "total_time": 0,
            "result": "",
            "accuracy": None,
            "error": str(e)
        }


def main(audio_path: str = None, reference: str = None):
    parser = argparse.ArgumentParser(description="Test whisper.cpp models")
    parser.add_argument("audio_file", nargs="?", help="Path to audio file")
    parser.add_argument("--reference", "-r", type=str, help="Reference transcript for accuracy measurement")

    # Only parse if called directly (not imported)
    if audio_path is None:
        args = parser.parse_args()
        audio_path = args.audio_file
        reference = args.reference

    if not audio_path:
        print("Usage: python tests/test_cpp.py <audio_file> --reference \"transcript\"")
        print()
        print("Example:")
        print("  python tests/test_cpp.py test_audio.wav --reference \"hello world\"")
        print()
        print(f"Models tested: {', '.join(MODELS)}")
        sys.exit(1)

    if not Path(audio_path).exists():
        print(f"Error: Audio file not found: {audio_path}")
        sys.exit(1)

    print(f"Audio file: {audio_path}")
    print(f"File size: {Path(audio_path).stat().st_size / 1024:.1f} KB")
    if reference:
        print(f"Reference: \"{reference}\"")
    print(f"Testing {len(MODELS)} whisper.cpp models...")

    results = []
    for model in MODELS:
        results.append(test_model(model, audio_path, reference))

    # Print comparison table
    print("\n")
    print("="*80)
    print("WHISPER.CPP MODEL COMPARISON")
    print("="*80)

    if reference:
        print(f"\n{'Model':<16} {'Transcribe':<12} {'Total':<10} {'Accuracy':<12} {'WER':<10}")
    else:
        print(f"\n{'Model':<16} {'Transcribe':<12} {'Total':<10} {'Status'}")
    print("-"*80)

    for r in results:
        if r["error"]:
            print(f"{r['model']:<16} ERROR: {r['error']}")
        elif reference and r["accuracy"]:
            print(f"{r['model']:<16} {r['transcribe_time']:.2f}s        {r['total_time']:.2f}s      {r['accuracy']['accuracy']:.1f}%        {r['accuracy']['wer']:.1%}")
        else:
            print(f"{r['model']:<16} {r['transcribe_time']:.2f}s        {r['total_time']:.2f}s      OK")

    print("\n" + "-"*80)
    print("TRANSCRIPTION OUTPUTS:")
    print("-"*80)

    for r in results:
        if not r["error"] and r["result"]:
            print(f"\n[{r['model']}]")
            print(f"  {r['result']}")

    # Find fastest and most accurate
    valid_results = [r for r in results if not r["error"] and r["result"]]
    if valid_results:
        fastest = min(valid_results, key=lambda x: x["transcribe_time"])
        print(f"\n{'='*80}")
        print(f"FASTEST: {fastest['model']} ({fastest['transcribe_time']:.2f}s)")

        if reference:
            accurate_results = [r for r in valid_results if r["accuracy"]]
            if accurate_results:
                most_accurate = max(accurate_results, key=lambda x: x["accuracy"]["accuracy"])
                print(f"MOST ACCURATE: {most_accurate['model']} ({most_accurate['accuracy']['accuracy']:.1f}%)")
        print(f"{'='*80}")

    return results


if __name__ == "__main__":
    main()
