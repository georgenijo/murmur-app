#!/usr/bin/env python3
"""Reject broadcast transports, including qualified calls and method aliases.

Tauri's Emitter trait and all emit transports belong exclusively to events.rs.
That adapter permits only literal WebviewWindow targets. Plain `emit(...)`
callbacks in capture sidecars are not Tauri broadcasts.
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


ADAPTER = Path('app/src-tauri/src/events.rs')


def violations(source: str, *, adapter: bool = False) -> list[int]:
    code = mask_literals(source)
    # Rust raw identifiers are the same identifiers, not an escape hatch.
    code = re.sub(r'\br#(?=[A-Za-z_])', '  ', code)
    # This exact helper publishes synthetic gesture tokens, not Tauri events.
    code = re.sub(r'\bcrate\s*::\s*audio_lifecycle\s*::\s*gesture\s*::\s*emit\b',
                  lambda m: ' ' * len(m.group()), code)
    patterns = [
        # No macro may hide the transport method behind an ident metavariable.
        r'(?:\.|::)\s*\$\s*[A-Za-z_]\w*',
        r'(?:\.|::)\s*(?:emit|emit_all|emit_str|emit_filter|emit_str_filter)\b',
        # Pinned Tauri 2.11.2 has Any, AnyLabel (including string conversions),
        # App, Window, Webview and WebviewWindow. Only the latter is sanctioned.
        r'\bEventTarget\s*::\s*(?!webview_window\b)[A-Za-z_]\w*',
    ]
    if not adapter:
        # Structural boundary: catches aliases, qualified imports, UFCS, function
        # pointers and macro invocations with a literal emit method argument.
        patterns += [r'\bEmitter\b', r'\b(?:emit_to|emit_str_to)\b']
    else:
        # Every adapter transport call must carry the exact constructor here.
        # Forbid pointers, aliases, dynamic targets and qualified alternative calls.
        patterns += [r'\b(?:emit_to|emit_str_to)\b(?!\s*\(\s*tauri\s*::\s*EventTarget\s*::\s*webview_window\s*\(\s*label\s*\)\s*,)']
    offsets = {m.start() for pattern in patterns for m in re.finditer(pattern, code)}
    return sorted({code.count('\n', 0, offset) + 1 for offset in offsets})


def main() -> int:
    failures = []
    for path in sorted((ROOT / 'app/src-tauri').rglob('*.rs')):
        if 'target' in path.relative_to(ROOT).parts:
            continue
        failures.extend(f'{path.relative_to(ROOT)}:{line}: broadcast transport' for line in violations(path.read_text(), adapter=path.relative_to(ROOT) == ADAPTER))
    if failures:
        print('\n'.join(failures), file=sys.stderr)
        return 1
    print('app/src-tauri: zero broadcast emits; Emitter confined to the constrained sink adapter')
    return 0


if __name__ == '__main__':
    sys.exit(main())
