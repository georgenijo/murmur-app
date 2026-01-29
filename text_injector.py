"""
Text Injector Module for Voice Dictation Tool

This module provides functionality to inject transcribed text into the currently
focused application using either clipboard paste or direct typing methods.
"""

import time
from pynput.keyboard import Controller, Key
import pyperclip


class TextInjector:
    """
    Injects text into the currently focused application.

    Supports two injection methods:
    - "paste": Copies text to clipboard and simulates Cmd+V (faster, recommended)
    - "type": Types text character by character (slower, but works in more contexts)
    """

    def __init__(self, method: str = "paste"):
        """
        Initialize the TextInjector.

        Args:
            method: Injection method to use. Either "paste" (default) or "type".

        Raises:
            ValueError: If an invalid method is specified.
        """
        if method not in ("paste", "type"):
            raise ValueError(f"Invalid injection method: {method}. Must be 'paste' or 'type'.")

        self.method = method
        self.keyboard = Controller()

    def _inject_via_paste(self, text: str) -> None:
        """
        Inject text by copying to clipboard and simulating Cmd+V.

        This method is faster and handles special characters well, but temporarily
        modifies the clipboard contents.

        Args:
            text: The text to inject.
        """
        # Store original clipboard content to restore later (optional)
        try:
            original_clipboard = pyperclip.paste()
        except Exception:
            original_clipboard = None

        # Copy text to clipboard
        pyperclip.copy(text)

        # Small delay to ensure clipboard is updated
        time.sleep(0.05)

        # Simulate Cmd+V (paste)
        with self.keyboard.pressed(Key.cmd):
            self.keyboard.press('v')
            self.keyboard.release('v')

        # Small delay to ensure paste completes
        time.sleep(0.05)

        # Optionally restore original clipboard content
        # Commented out by default as it may interfere with rapid dictation
        # if original_clipboard is not None:
        #     time.sleep(0.1)
        #     pyperclip.copy(original_clipboard)

    def _inject_via_typing(self, text: str) -> None:
        """
        Inject text by typing each character directly.

        This method is slower but doesn't modify the clipboard and works in
        contexts where paste might not be supported.

        Args:
            text: The text to inject.
        """
        # Small delay before typing begins
        time.sleep(0.05)

        # Type the text character by character
        self.keyboard.type(text)

        # Small delay after typing completes
        time.sleep(0.05)

    def inject(self, text: str) -> None:
        """
        Inject text into the currently focused application.

        Uses the injection method specified during initialization.

        Args:
            text: The text to inject.
        """
        if not text:
            return

        if self.method == "paste":
            self._inject_via_paste(text)
        else:
            self._inject_via_typing(text)


if __name__ == "__main__":
    # Test block for the TextInjector
    print("Text Injector Test")
    print("=" * 40)
    print()
    print("This will test both injection methods.")
    print("You have 3 seconds to focus a text field...")
    print()

    # Countdown
    for i in range(3, 0, -1):
        print(f"  {i}...")
        time.sleep(1)

    print()
    print("Injecting text via PASTE method...")

    # Test paste method
    paste_injector = TextInjector(method="paste")
    paste_injector.inject("Hello from paste method! ")

    time.sleep(0.5)

    print("Injecting text via TYPE method...")

    # Test type method
    type_injector = TextInjector(method="type")
    type_injector.inject("Hello from type method!")

    print()
    print("Test complete!")
    print("You should see both test messages in your focused text field.")
