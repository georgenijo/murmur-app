#!/usr/bin/env python3
"""
Record a test audio file for evaluation.

Usage:
    python record_test.py [duration_seconds]

Default: 15 seconds
"""

import sys
import time
from pathlib import Path

from audio_recorder import AudioRecorder


def main():
    duration = int(sys.argv[1]) if len(sys.argv) > 1 else 15
    output_file = Path("test_audio.wav")

    print(f"Recording for {duration} seconds")
    print("Starting in 3 seconds...")
    time.sleep(1)
    print("3...")
    time.sleep(1)
    print("2...")
    time.sleep(1)
    print("1...")
    print()
    print(">>> RECORDING NOW - SPEAK! <<<")
    print()

    recorder = AudioRecorder()
    recorder.start_recording()

    # Countdown during recording
    for i in range(duration, 0, -1):
        print(f"  {i}s remaining...")
        time.sleep(1)

    temp_file = recorder.stop_recording()

    # Copy to project directory
    import shutil
    shutil.copy(temp_file, output_file)
    temp_file.unlink()

    print()
    print(f">>> DONE <<<")
    print(f"Saved to: {output_file.absolute()}")
    print(f"Size: {output_file.stat().st_size / 1024:.1f} KB")
    print()
    print("Now run:")
    print(f"  python eval.py {output_file}")


if __name__ == "__main__":
    main()
