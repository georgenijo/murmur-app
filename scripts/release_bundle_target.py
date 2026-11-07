#!/usr/bin/env python3
"""Fail closed before any release job receives secrets."""
import os


def validate_target(event: str, target: str) -> str:
    if target not in {"tauri", "gpui"}:
        raise ValueError("unknown bundle_target")
    if target == "gpui" and event != "workflow_dispatch":
        raise ValueError("GPUI is restricted to non-publishing manual rehearsals")
    return target


if __name__ == "__main__":
    print(validate_target(os.environ["GITHUB_EVENT_NAME"], os.environ["BUNDLE_TARGET"]))
