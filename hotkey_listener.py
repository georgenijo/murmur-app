"""
Hotkey listener module for global hotkey detection with hold-to-record support.

Uses pynput for cross-platform global key detection.
"""

import threading
from typing import Callable, Optional

from pynput import keyboard
from pynput.keyboard import Key, KeyCode


class HotkeyListener:
    """
    Global hotkey listener with hold-to-record support.

    Detects when a configured hotkey is pressed and released,
    triggering callbacks for each event. Callbacks run in separate
    daemon threads to avoid blocking the listener.
    """

    # Key mapping from string names to pynput key objects
    KEY_MAP = {
        # Alt/Option keys
        "alt_r": Key.alt_r,
        "alt_l": Key.alt_l,
        "option_r": Key.alt_r,  # macOS alias
        "option_l": Key.alt_l,  # macOS alias

        # Control keys
        "ctrl_r": Key.ctrl_r,
        "ctrl_l": Key.ctrl_l,
        "control_r": Key.ctrl_r,
        "control_l": Key.ctrl_l,

        # Shift keys
        "shift_r": Key.shift_r,
        "shift_l": Key.shift_l,

        # Command keys (macOS)
        "cmd_r": Key.cmd_r,
        "cmd_l": Key.cmd_l,
        "command_r": Key.cmd_r,
        "command_l": Key.cmd_l,

        # Function keys
        "f1": Key.f1,
        "f2": Key.f2,
        "f3": Key.f3,
        "f4": Key.f4,
        "f5": Key.f5,
        "f6": Key.f6,
        "f7": Key.f7,
        "f8": Key.f8,
        "f9": Key.f9,
        "f10": Key.f10,
        "f11": Key.f11,
        "f12": Key.f12,
        "f13": Key.f13,
        "f14": Key.f14,
        "f15": Key.f15,
        "f16": Key.f16,
        "f17": Key.f17,
        "f18": Key.f18,
        "f19": Key.f19,
        "f20": Key.f20,

        # Special keys
        "caps_lock": Key.caps_lock,
        "space": Key.space,
        "tab": Key.tab,
        "enter": Key.enter,
        "return": Key.enter,
        "escape": Key.esc,
        "esc": Key.esc,
        "backspace": Key.backspace,
        "delete": Key.delete,

        # Navigation keys
        "home": Key.home,
        "end": Key.end,
        "page_up": Key.page_up,
        "page_down": Key.page_down,
    }

    def __init__(
        self,
        on_press: Callable[[], None],
        on_release: Callable[[], None],
        hotkey: str = "alt_r"
    ):
        """
        Initialize the hotkey listener.

        Args:
            on_press: Callback function to call when hotkey is pressed.
            on_release: Callback function to call when hotkey is released.
            hotkey: String name of the hotkey to listen for (default: "alt_r").
                   See KEY_MAP for available key names.

        Raises:
            ValueError: If the hotkey name is not recognized.
        """
        self.on_press = on_press
        self.on_release = on_release
        self.hotkey = hotkey.lower()

        # Resolve the hotkey to a pynput key object
        if self.hotkey in self.KEY_MAP:
            self._target_key = self.KEY_MAP[self.hotkey]
        else:
            raise ValueError(
                f"Unknown hotkey: '{hotkey}'. "
                f"Available keys: {', '.join(sorted(self.KEY_MAP.keys()))}"
            )

        # State tracking
        self._is_pressed = False
        self._listener: Optional[keyboard.Listener] = None
        self._lock = threading.Lock()

    def _matches_target(self, key) -> bool:
        """Check if the pressed key matches our target hotkey."""
        return key == self._target_key

    def _handle_press(self, key):
        """Handle key press events."""
        if not self._matches_target(key):
            return

        with self._lock:
            if self._is_pressed:
                # Already pressed, avoid duplicate triggers
                return
            self._is_pressed = True

        # Run callback in a separate daemon thread to not block the listener
        thread = threading.Thread(target=self.on_press, daemon=True)
        thread.start()

    def _handle_release(self, key):
        """Handle key release events."""
        if not self._matches_target(key):
            return

        with self._lock:
            if not self._is_pressed:
                # Not pressed, nothing to release
                return
            self._is_pressed = False

        # Run callback in a separate daemon thread to not block the listener
        thread = threading.Thread(target=self.on_release, daemon=True)
        thread.start()

    def start(self):
        """Start listening for hotkey events."""
        if self._listener is not None:
            return  # Already running

        self._listener = keyboard.Listener(
            on_press=self._handle_press,
            on_release=self._handle_release
        )
        self._listener.start()

    def stop(self):
        """Stop listening for hotkey events."""
        if self._listener is None:
            return

        self._listener.stop()
        self._listener = None

        # Reset state
        with self._lock:
            self._is_pressed = False

    def join(self):
        """Wait for the listener thread to finish."""
        if self._listener is not None:
            self._listener.join()

    @property
    def is_pressed(self) -> bool:
        """Check if the hotkey is currently pressed."""
        with self._lock:
            return self._is_pressed

    @classmethod
    def available_keys(cls) -> list[str]:
        """Return a list of available hotkey names."""
        return sorted(cls.KEY_MAP.keys())


if __name__ == "__main__":
    import time

    print("=" * 60)
    print("Hotkey Listener Test")
    print("=" * 60)
    print()
    print("Instructions:")
    print("  - Press and hold the Right Option key to test")
    print("  - Release the key to see the release message")
    print("  - Press Ctrl+C to exit")
    print()
    print("Available hotkeys:", ", ".join(HotkeyListener.available_keys()))
    print()
    print("Listening for Right Option key (alt_r)...")
    print("-" * 60)

    def on_press():
        print("[PRESS] Right Option key pressed - recording would start here")

    def on_release():
        print("[RELEASE] Right Option key released - recording would stop here")

    listener = HotkeyListener(
        on_press=on_press,
        on_release=on_release,
        hotkey="alt_r"
    )

    try:
        listener.start()
        # Keep the main thread alive
        while True:
            time.sleep(0.1)
    except KeyboardInterrupt:
        print()
        print("-" * 60)
        print("Stopping listener...")
        listener.stop()
        print("Done.")
