"""
Whisper Transcription Module for Voice Dictation Tool

This module provides audio transcription using OpenAI's open-source Whisper model.

Model Options:
    - tiny.en: Fastest, lowest accuracy (~39M parameters)
    - base.en: Good balance of speed and accuracy (~74M parameters)
    - small.en: Better accuracy, slower (~244M parameters)
    - turbo: Optimized for speed with good accuracy

The .en models are English-only and perform better for English transcription.
"""

import sys
import whisper


class WhisperTranscriber:
    """
    A transcriber class that uses OpenAI's Whisper model for speech-to-text.

    The model is loaded once during initialization and reused for all
    transcription requests to improve performance.

    Attributes:
        model_name (str): Name of the Whisper model being used.
        model: The loaded Whisper model instance.

    Model Options:
        - tiny.en: ~39M parameters, fastest, lowest accuracy
        - base.en: ~74M parameters, good balance (default)
        - small.en: ~244M parameters, better accuracy, slower
        - turbo: Optimized for speed with good accuracy
    """

    def __init__(self, model_name: str = "base.en"):
        """
        Initialize the WhisperTranscriber with a specified model.

        Args:
            model_name: Name of the Whisper model to load. Options include:
                - "tiny.en": Fastest, lowest accuracy
                - "base.en": Good balance of speed and accuracy (default)
                - "small.en": Better accuracy, slower
                - "turbo": Optimized for speed with good accuracy
        """
        self.model_name = model_name
        self.model = None
        self._load_model()

    def _load_model(self):
        """Load the Whisper model into memory."""
        try:
            print(f"Loading Whisper model '{self.model_name}'...")
            self.model = whisper.load_model(self.model_name)
            print(f"Model '{self.model_name}' loaded successfully.")
        except Exception as e:
            print(f"Error loading Whisper model: {e}")
            self.model = None

    def transcribe(self, audio_path: str, language: str = "en") -> str:
        """
        Transcribe an audio file to text.

        Args:
            audio_path: Path to the audio file to transcribe.
            language: Language code for transcription (default: "en").

        Returns:
            The transcribed text as a string. Returns an empty string if
            transcription fails or if the model is not loaded.
        """
        if self.model is None:
            print("Error: Model not loaded. Cannot transcribe.")
            return ""

        try:
            # Convert Path to string if needed
            audio_path_str = str(audio_path)
            result = self.model.transcribe(
                audio_path_str,
                language=language,
                fp16=False  # Use fp32 for CPU compatibility
            )
            return result.get("text", "").strip()
        except FileNotFoundError:
            print(f"Error: Audio file not found: {audio_path}")
            return ""
        except Exception as e:
            print(f"Error during transcription: {e}")
            return ""


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python transcriber.py <audio_file_path> [model_name]")
        print()
        print("Arguments:")
        print("  audio_file_path  Path to the audio file to transcribe")
        print("  model_name       Optional: Whisper model to use (default: base.en)")
        print()
        print("Available models:")
        print("  tiny.en  - Fastest, lowest accuracy")
        print("  base.en  - Good balance of speed and accuracy")
        print("  small.en - Better accuracy, slower")
        print("  turbo    - Optimized for speed with good accuracy")
        sys.exit(1)

    audio_file = sys.argv[1]
    model_name = sys.argv[2] if len(sys.argv) > 2 else "base.en"

    transcriber = WhisperTranscriber(model_name=model_name)
    transcribed_text = transcriber.transcribe(audio_file)

    if transcribed_text:
        print()
        print("Transcription:")
        print("-" * 40)
        print(transcribed_text)
    else:
        print("No transcription result.")
