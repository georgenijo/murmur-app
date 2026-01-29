"""
Deepgram Transcription Module for Voice Dictation Tool

Uses Deepgram's API for fast, accurate cloud-based transcription.
Requires DEEPGRAM_API_KEY environment variable.

Free tier: 12,000 minutes (200 hours)
"""

import os
from pathlib import Path

from dotenv import load_dotenv
from deepgram import DeepgramClient

# Load .env file from project directory
load_dotenv(Path(__file__).parent / ".env")


class DeepgramTranscriber:
    """
    A transcriber class that uses Deepgram's API for speech-to-text.

    Requires DEEPGRAM_API_KEY environment variable to be set.
    """

    # Model mapping
    MODEL_MAP = {
        # Deepgram models
        "nova-3": "nova-3",
        "nova-2": "nova-2",
        "nova": "nova",
        "enhanced": "enhanced",
        "base": "base",
        # Map Whisper model names to Deepgram equivalents
        "turbo": "nova-2",
        "large-v3": "nova-2",
        "small.en": "nova",
        "base.en": "enhanced",
        "tiny.en": "base",
    }

    def __init__(self, model_name: str = "nova-2"):
        """
        Initialize the DeepgramTranscriber with a specified model.

        Args:
            model_name: Name of the Deepgram model to use.
                Options: nova-3, nova-2, nova, enhanced, base
                Also accepts Whisper model names (mapped to Deepgram equivalents)
        """
        self.model_name = self.MODEL_MAP.get(model_name, "nova-2")
        self.client = None
        self._load_model()

    def _load_model(self):
        """Initialize the Deepgram client."""
        api_key = os.environ.get("DEEPGRAM_API_KEY")

        if not api_key:
            print("Error: DEEPGRAM_API_KEY environment variable not set")
            print("Get your free API key at: https://console.deepgram.com/signup")
            print("Then run: export DEEPGRAM_API_KEY='your-key-here'")
            self.client = None
            return

        try:
            print(f"Initializing Deepgram client (model: {self.model_name})...")
            self.client = DeepgramClient(api_key=api_key)
            print(f"Deepgram client ready (model: {self.model_name})")
        except Exception as e:
            print(f"Error initializing Deepgram client: {e}")
            self.client = None

    def transcribe(self, audio_path: str, language: str = "en") -> str:
        """
        Transcribe an audio file to text using Deepgram API.

        Args:
            audio_path: Path to the audio file to transcribe.
            language: Language code for transcription (default: "en").

        Returns:
            The transcribed text as a string.
        """
        if self.client is None:
            print("Error: Deepgram client not initialized. Check your API key.")
            return ""

        try:
            # Convert Path to string if needed
            audio_path_str = str(audio_path)

            # Read the audio file
            with open(audio_path_str, "rb") as f:
                audio_data = f.read()

            # Transcribe using Deepgram API
            response = self.client.listen.v1.media.transcribe_file(
                request=audio_data,
                model=self.model_name,
                language=language,
                smart_format=True,
                punctuate=True,
            )

            # Extract transcript from response
            transcript = response.results.channels[0].alternatives[0].transcript

            return transcript.strip()

        except FileNotFoundError:
            print(f"Error: Audio file not found: {audio_path}")
            return ""
        except Exception as e:
            print(f"Error during Deepgram transcription: {e}")
            return ""


if __name__ == "__main__":
    import sys

    if len(sys.argv) < 2:
        print("Usage: python transcriber_deepgram.py <audio_file_path> [model_name]")
        print()
        print("Requires: export DEEPGRAM_API_KEY='your-key-here'")
        print()
        print("Available models: nova-3, nova-2, nova, enhanced, base")
        sys.exit(1)

    audio_file = sys.argv[1]
    model_name = sys.argv[2] if len(sys.argv) > 2 else "nova-2"

    transcriber = DeepgramTranscriber(model_name=model_name)
    transcribed_text = transcriber.transcribe(audio_file)

    if transcribed_text:
        print()
        print("Transcription:")
        print("-" * 40)
        print(transcribed_text)
    else:
        print("No transcription result.")
