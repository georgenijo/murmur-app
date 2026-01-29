"""
Whisper.cpp Transcription Module for Voice Dictation Tool

Uses whisper.cpp via pywhispercpp for faster local transcription.
This is 2-4x faster than OpenAI's Python implementation.
"""

from pywhispercpp.model import Model
from pathlib import Path


class WhisperCppTranscriber:
    """
    A transcriber class that uses whisper.cpp for speech-to-text.

    Faster than OpenAI's Python implementation, optimized for Apple Silicon.
    """

    # Model name mapping to ggml model names
    MODEL_MAP = {
        "tiny.en": "tiny.en",
        "tiny": "tiny",
        "base.en": "base.en",
        "base": "base",
        "small.en": "small.en",
        "small": "small",
        "medium.en": "medium.en",
        "medium": "medium",
        "large": "large-v3",
        "large-v3": "large-v3",
        "turbo": "large-v3-turbo",
    }

    def __init__(self, model_name: str = "base.en"):
        """
        Initialize the WhisperCppTranscriber with a specified model.

        Args:
            model_name: Name of the Whisper model to load.
        """
        self.model_name = model_name
        self.model = None
        self._load_model()

    def _load_model(self):
        """Load the whisper.cpp model."""
        try:
            # Map model name to ggml model name
            ggml_model = self.MODEL_MAP.get(self.model_name, self.model_name)

            print(f"Loading whisper.cpp model '{ggml_model}'...")
            # Model will be downloaded automatically if not present
            self.model = Model(ggml_model, n_threads=8)
            print(f"Model '{ggml_model}' loaded successfully (whisper.cpp)")
        except Exception as e:
            print(f"Error loading whisper.cpp model: {e}")
            self.model = None

    def transcribe(self, audio_path: str, language: str = "en") -> str:
        """
        Transcribe an audio file to text.

        Args:
            audio_path: Path to the audio file to transcribe.
            language: Language code for transcription (default: "en").

        Returns:
            The transcribed text as a string.
        """
        if self.model is None:
            print("Error: Model not loaded. Cannot transcribe.")
            return ""

        try:
            # Convert Path to string if needed
            audio_path_str = str(audio_path)

            # Transcribe using whisper.cpp
            segments = self.model.transcribe(audio_path_str, language=language)

            # Combine all segments into a single string
            text = " ".join(segment.text for segment in segments).strip()

            return text
        except FileNotFoundError:
            print(f"Error: Audio file not found: {audio_path}")
            return ""
        except Exception as e:
            print(f"Error during transcription: {e}")
            return ""


if __name__ == "__main__":
    import sys

    if len(sys.argv) < 2:
        print("Usage: python transcriber_cpp.py <audio_file_path> [model_name]")
        print()
        print("Available models: tiny.en, base.en, small.en, medium.en, large-v3, turbo")
        sys.exit(1)

    audio_file = sys.argv[1]
    model_name = sys.argv[2] if len(sys.argv) > 2 else "base.en"

    transcriber = WhisperCppTranscriber(model_name=model_name)
    transcribed_text = transcriber.transcribe(audio_file)

    if transcribed_text:
        print()
        print("Transcription:")
        print("-" * 40)
        print(transcribed_text)
    else:
        print("No transcription result.")
