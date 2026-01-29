#!/usr/bin/env python3
"""
Run all transcription backend tests with accuracy measurement.

Usage:
    python tests/run_all.py <audio_file> --reference "the actual words spoken"
    python tests/run_all.py test_audio.wav --reference "hello world this is a test"
"""

import argparse
import sys
from pathlib import Path

# Add project root to path
sys.path.insert(0, str(Path(__file__).parent.parent))
sys.path.insert(0, str(Path(__file__).parent))

from dotenv import load_dotenv
load_dotenv(Path(__file__).parent.parent / ".env")


def main():
    parser = argparse.ArgumentParser(description="Run all transcription backend tests")
    parser.add_argument("audio_file", nargs="?", help="Path to audio file")
    parser.add_argument("--reference", "-r", type=str, help="Reference transcript for accuracy measurement")

    args = parser.parse_args()

    if not args.audio_file:
        print("Usage: python tests/run_all.py <audio_file> --reference \"transcript\"")
        print()
        print("This runs all backend tests:")
        print("  - OpenAI Whisper: tiny.en, base.en, small.en, turbo")
        print("  - whisper.cpp: tiny.en, base.en, small.en, medium.en, large-v3, large-v3-turbo")
        print("  - Deepgram: nova-3, nova-2, nova, enhanced, base")
        print()
        print("Add --reference to measure accuracy against ground truth.")
        sys.exit(1)

    audio_path = args.audio_file
    reference = args.reference

    if not Path(audio_path).exists():
        print(f"Error: Audio file not found: {audio_path}")
        sys.exit(1)

    # Inject audio_path into sys.argv for submodules
    sys.argv = [sys.argv[0], audio_path]
    if reference:
        sys.argv.extend(["--reference", reference])

    all_results = {}

    # Test OpenAI Whisper
    print("\n" + "="*80)
    print("TESTING OPENAI WHISPER BACKEND")
    print("="*80)
    try:
        from test_openai import main as test_openai
        all_results["openai"] = test_openai(audio_path, reference)
    except Exception as e:
        print(f"OpenAI Whisper tests failed: {e}")
        all_results["openai"] = []

    # Test whisper.cpp
    print("\n" + "="*80)
    print("TESTING WHISPER.CPP BACKEND")
    print("="*80)
    try:
        from test_cpp import main as test_cpp
        all_results["cpp"] = test_cpp(audio_path, reference)
    except Exception as e:
        print(f"whisper.cpp tests failed: {e}")
        all_results["cpp"] = []

    # Test Deepgram
    print("\n" + "="*80)
    print("TESTING DEEPGRAM BACKEND")
    print("="*80)
    try:
        from test_deepgram import main as test_deepgram
        all_results["deepgram"] = test_deepgram(audio_path, reference)
    except Exception as e:
        print(f"Deepgram tests failed: {e}")
        all_results["deepgram"] = []

    # Final summary
    print("\n")
    print("="*80)
    print("FINAL SUMMARY - ALL BACKENDS")
    print("="*80)

    if reference:
        print(f"\n{'Backend':<12} {'Model':<16} {'Transcribe':<12} {'Total':<10} {'Accuracy':<10} {'WER':<10}")
    else:
        print(f"\n{'Backend':<12} {'Model':<16} {'Transcribe':<12} {'Total':<10}")
    print("-"*80)

    all_valid = []
    for backend, results in all_results.items():
        for r in results:
            if not r.get("error"):
                all_valid.append({**r, "backend": backend})
                if reference and r.get("accuracy"):
                    print(f"{backend:<12} {r['model']:<16} {r['transcribe_time']:.2f}s        {r['total_time']:.2f}s      {r['accuracy']['accuracy']:.1f}%       {r['accuracy']['wer']:.1%}")
                else:
                    print(f"{backend:<12} {r['model']:<16} {r['transcribe_time']:.2f}s        {r['total_time']:.2f}s")

    if all_valid:
        print(f"\n{'='*80}")
        fastest = min(all_valid, key=lambda x: x["transcribe_time"])
        print(f"OVERALL FASTEST: {fastest['backend']} / {fastest['model']} ({fastest['transcribe_time']:.2f}s)")

        if reference:
            accurate_results = [r for r in all_valid if r.get("accuracy")]
            if accurate_results:
                most_accurate = max(accurate_results, key=lambda x: x["accuracy"]["accuracy"])
                print(f"OVERALL MOST ACCURATE: {most_accurate['backend']} / {most_accurate['model']} ({most_accurate['accuracy']['accuracy']:.1f}%)")

                # Best balance (accuracy / time ratio)
                for r in accurate_results:
                    r["efficiency"] = r["accuracy"]["accuracy"] / max(r["transcribe_time"], 0.01)
                best_balance = max(accurate_results, key=lambda x: x["efficiency"])
                print(f"BEST BALANCE (accuracy/speed): {best_balance['backend']} / {best_balance['model']} ({best_balance['accuracy']['accuracy']:.1f}% in {best_balance['transcribe_time']:.2f}s)")

        print(f"{'='*80}")


if __name__ == "__main__":
    main()
