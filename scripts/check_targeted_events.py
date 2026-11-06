#!/usr/bin/env python3
"""Reject broadcast transports, including qualified calls and method aliases.

Plain `emit(...)` callbacks in capture sidecars are not Tauri broadcasts.
"""
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]


def mask_literals(source: str) -> str:
    # Preserve offsets/newlines for diagnostics, including nested Rust comments.
    pattern = re.compile(r'//[^\n]*|/\*|r(#+)?"|"|\'(?:\\.|[^\'\\\n])\'')
    chars = list(source)
    position = 0
    while match := pattern.search(source, position):
        start, end = match.span()
        token = match.group()
        if token == '/*':
            depth = 1
            while depth and end < len(source):
                if source.startswith('/*', end):
                    depth += 1
                    end += 2
                elif source.startswith('*/', end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
        elif token.startswith('r'):
            terminator = '"' + (match.group(1) or '')
            close = source.find(terminator, end)
            end = len(source) if close < 0 else close + len(terminator)
        elif token == '"':
            while end < len(source):
                if source[end] == '\\':
                    end += 2
                elif source[end] == '"':
                    end += 1
                    break
                else:
                    end += 1
        for i in range(start, min(end, len(chars))):
            if chars[i] != '\n':
                chars[i] = ' '
        position = end
    return ''.join(chars)


def violations(source: str) -> list[int]:
    code = mask_literals(source)
    # This fully qualified helper publishes only a synthetic gesture token into
    # the capture correlation ledger; it is not a Tauri emitter.
    code = re.sub(r'\bcrate\s*::\s*audio_lifecycle\s*::\s*gesture\s*::\s*emit\b',
                  lambda m: ' ' * len(m.group()), code)
    # Match names even when taken as function pointers or called with turbofish.
    forbidden = r'(?:\.|::)\s*(?:emit|emit_str|emit_filter|emit_str_filter)\b|\bemit_all\b'
    return [code.count('\n', 0, m.start()) + 1 for m in re.finditer(forbidden, code)]


def main() -> int:
    failures = []
    for path in sorted((ROOT / 'app/src-tauri').rglob('*.rs')):
        if 'target' in path.relative_to(ROOT).parts:
            continue
        failures.extend(f'{path.relative_to(ROOT)}:{line}: broadcast transport' for line in violations(path.read_text()))
    if failures:
        print('\n'.join(failures), file=sys.stderr)
        return 1
    print('app/src-tauri: zero broadcast emits (methods, qualified calls, aliases)')
    return 0


if __name__ == '__main__':
    sys.exit(main())
