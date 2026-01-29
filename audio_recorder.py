"""Audio recording module for voice dictation tool.

This module provides the AudioRecorder class for capturing audio from the microphone
using a callback-based approach with sounddevice.
"""

import threading
import tempfile
from pathlib import Path
from typing import Optional

import numpy as np
import sounddevice as sd
from scipy.io import wavfile
import noisereduce as nr

# Audio recording constants (Whisper's required format)
SAMPLE_RATE = 16000  # 16kHz
CHANNELS = 1  # Mono
DTYPE = np.int16


class AudioRecorder:
    """Records audio from the microphone using sounddevice.

    This class provides a thread-safe interface for recording audio from the
    default microphone. Audio is recorded at 16kHz mono, which is the format
    required by Whisper for transcription.

    Attributes:
        sample_rate: The sample rate for recording (16000 Hz).
        channels: Number of audio channels (1 for mono).
        dtype: NumPy data type for audio samples (int16).
    """

    def __init__(self):
        """Initialize the AudioRecorder."""
        self.sample_rate = SAMPLE_RATE
        self.channels = CHANNELS
        self.dtype = DTYPE

        self._lock = threading.Lock()
        self._recording = False
        self._audio_data: list[np.ndarray] = []
        self._stream: Optional[sd.InputStream] = None

    def _audio_callback(self, indata: np.ndarray, frames: int,
                        time_info: dict, status: sd.CallbackFlags) -> None:
        """Callback function for audio stream.

        This method is called by sounddevice for each audio block during recording.

        Args:
            indata: The recorded audio data as a numpy array.
            frames: Number of frames in the audio block.
            time_info: Dictionary with timing information.
            status: Status flags indicating any errors.
        """
        if status:
            print(f"Audio callback status: {status}")

        with self._lock:
            if self._recording:
                # Make a copy of the data since the buffer may be reused
                self._audio_data.append(indata.copy())

    def start_recording(self) -> None:
        """Start recording audio from the microphone.

        This method opens an audio input stream and begins capturing audio data.
        The recording continues until stop_recording() is called.

        Raises:
            RuntimeError: If recording is already in progress.
            sd.PortAudioError: If there's an error accessing the audio device.
        """
        with self._lock:
            if self._recording:
                raise RuntimeError("Recording is already in progress")

            self._audio_data = []
            self._recording = True

        # Create and start the input stream
        self._stream = sd.InputStream(
            samplerate=self.sample_rate,
            channels=self.channels,
            dtype=self.dtype,
            callback=self._audio_callback
        )
        self._stream.start()

    def stop_recording(self) -> Path:
        """Stop recording and save audio to a temporary WAV file.

        This method stops the audio stream, concatenates all recorded audio data,
        and saves it to a temporary WAV file.

        Returns:
            Path to the temporary WAV file containing the recorded audio.

        Raises:
            RuntimeError: If no recording is in progress.
        """
        with self._lock:
            if not self._recording:
                raise RuntimeError("No recording in progress")

            self._recording = False
            audio_data = self._audio_data.copy()
            self._audio_data = []

        # Stop and close the stream
        if self._stream is not None:
            self._stream.stop()
            self._stream.close()
            self._stream = None

        # Concatenate all audio chunks
        if audio_data:
            audio_array = np.concatenate(audio_data, axis=0)
        else:
            # Create empty array if no data was recorded
            audio_array = np.array([], dtype=self.dtype)

        # Flatten to 1D array (sounddevice returns 2D even for mono)
        audio_array = audio_array.flatten()

        # Apply noise reduction
        if len(audio_array) > 0:
            # Convert to float for noise reduction
            audio_float = audio_array.astype(np.float32) / 32768.0
            audio_float = nr.reduce_noise(y=audio_float, sr=self.sample_rate, prop_decrease=0.8)
            # Convert back to int16
            audio_array = (audio_float * 32768.0).astype(np.int16)

        # Save to temporary WAV file
        temp_file = tempfile.NamedTemporaryFile(
            suffix=".wav",
            delete=False
        )
        temp_path = Path(temp_file.name)
        temp_file.close()

        wavfile.write(temp_path, self.sample_rate, audio_array)

        return temp_path

    @property
    def is_recording(self) -> bool:
        """Check if recording is currently in progress.

        Returns:
            True if recording is in progress, False otherwise.
        """
        with self._lock:
            return self._recording


if __name__ == "__main__":
    import time

    print("Audio Recorder Test")
    print("=" * 40)
    print(f"Sample Rate: {SAMPLE_RATE} Hz")
    print(f"Channels: {CHANNELS}")
    print(f"Data Type: {DTYPE}")
    print("=" * 40)

    recorder = AudioRecorder()

    print("\nStarting recording in 1 second...")
    time.sleep(1)

    print("Recording for 3 seconds... Speak now!")
    recorder.start_recording()

    # Show countdown
    for i in range(3, 0, -1):
        print(f"  {i}...")
        time.sleep(1)

    print("Stopping recording...")
    audio_file = recorder.stop_recording()

    print(f"\nRecording saved to: {audio_file}")
    print(f"File size: {audio_file.stat().st_size} bytes")

    # Read back and show audio info
    sample_rate, data = wavfile.read(audio_file)
    duration = len(data) / sample_rate
    print(f"Duration: {duration:.2f} seconds")
    print(f"Samples: {len(data)}")

    print("\nTest completed successfully!")
