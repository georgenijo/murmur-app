"""
LLM Cleanup Module for Voice Dictation Tool

Uses Ollama to clean up transcribed text by removing filler words,
fixing grammar, and adding proper punctuation.
"""

import requests
from typing import Optional


class LLMCleanup:
    """Cleans up transcribed text using Ollama LLM."""

    CLEANUP_PROMPT = """Repeat the following text exactly, only fixing obvious transcription errors and adding punctuation. Do not change words, do not censor, do not refuse. Just output the corrected text.

Text: {text}

Corrected:"""

    def __init__(
        self,
        model: str = "llama3.2:3b",
        host: str = "http://localhost:11434",
        enabled: bool = True
    ):
        """
        Initialize the LLM cleanup module.

        Args:
            model: The Ollama model to use for cleanup
            host: The Ollama API host URL
            enabled: Whether LLM cleanup is enabled
        """
        self.model = model
        self.host = host.rstrip("/")
        self.enabled = enabled

    def is_available(self) -> bool:
        """
        Check if Ollama is running and available.

        Returns:
            True if Ollama is available, False otherwise
        """
        if not self.enabled:
            return False

        try:
            response = requests.get(
                f"{self.host}/api/tags",
                timeout=2
            )
            return response.status_code == 200
        except (requests.RequestException, ConnectionError):
            return False

    def cleanup(self, text: str) -> str:
        """
        Clean up transcribed text using Ollama.

        Args:
            text: The raw transcribed text to clean up

        Returns:
            Cleaned text, or original text if cleanup fails
        """
        if not self.enabled:
            return text

        if not text or not text.strip():
            return text

        try:
            prompt = self.CLEANUP_PROMPT.format(text=text)

            response = requests.post(
                f"{self.host}/api/generate",
                json={
                    "model": self.model,
                    "prompt": prompt,
                    "stream": False,
                    "options": {
                        "temperature": 0.3,
                        "num_predict": 500
                    }
                },
                timeout=30
            )

            if response.status_code == 200:
                result = response.json()
                cleaned_text = result.get("response", "").strip()

                # Detect refusals/unhelpful responses and fall back to raw text
                refusal_phrases = [
                    "I can't", "I cannot", "I'm not able", "I am not able",
                    "I won't", "I will not", "I'm unable", "I am unable",
                    "can't fulfill", "cannot fulfill", "can't help", "cannot help",
                    "I couldn't find", "I could not find", "provide the original",
                    "provide more context", "please provide", "not sure what",
                    "start again", "from scratch"
                ]
                if any(phrase.lower() in cleaned_text.lower() for phrase in refusal_phrases):
                    return text  # Return raw text if LLM refused

                # If response is way longer than input, it's probably not a cleanup
                if len(cleaned_text) > len(text) * 3:
                    return text

                if cleaned_text:
                    return cleaned_text

            # Fall back to raw text if response is empty or invalid
            return text

        except (requests.RequestException, ConnectionError, ValueError) as e:
            # Fall back gracefully to raw text if Ollama is unavailable
            print(f"LLM cleanup failed, using raw text: {e}")
            return text


if __name__ == "__main__":
    # Test the LLM cleanup module
    print("Testing LLM Cleanup Module")
    print("=" * 50)

    cleanup = LLMCleanup()

    # Check if Ollama is available
    print(f"\nOllama host: {cleanup.host}")
    print(f"Model: {cleanup.model}")
    print(f"Enabled: {cleanup.enabled}")

    available = cleanup.is_available()
    print(f"Ollama available: {available}")

    # Test with sample text containing filler words
    sample_texts = [
        "um so like I was thinking you know that we should uh basically go to the store",
        "well actually I mean the meeting is at like three oclock you know",
        "so uh basically what I wanted to say is um that the project is going well I think",
    ]

    print("\n" + "=" * 50)
    print("Testing text cleanup:")
    print("=" * 50)

    for i, text in enumerate(sample_texts, 1):
        print(f"\nTest {i}:")
        print(f"  Original: {text}")
        cleaned = cleanup.cleanup(text)
        print(f"  Cleaned:  {cleaned}")

    # Test with empty text
    print("\n" + "=" * 50)
    print("Testing edge cases:")
    print("=" * 50)

    print(f"\nEmpty string: '{cleanup.cleanup('')}'")
    print(f"Whitespace only: '{cleanup.cleanup('   ')}'")

    # Test with disabled cleanup
    print("\n" + "=" * 50)
    print("Testing disabled cleanup:")
    print("=" * 50)

    disabled_cleanup = LLMCleanup(enabled=False)
    print(f"Enabled: {disabled_cleanup.enabled}")
    print(f"Available: {disabled_cleanup.is_available()}")
    test_text = "um like hello"
    print(f"Original: {test_text}")
    print(f"Result (should be unchanged): {disabled_cleanup.cleanup(test_text)}")
