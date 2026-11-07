#!/usr/bin/env python3
"""Enforce the repository's source-level Tauri transport boundary.

Reserve all emit/emit_* identifier tokens for one constrained adapter. This is
not a Rust compiler or a sandbox for arbitrary proc-macro/build-script output.
"""
import ast
import os
from pathlib import Path
import re
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
ADAPTER = Path('app/src-tauri/src/events.rs')
GPUI_ADAPTER = Path('app/gpui/src/events.rs')
# Exact repository-relative build/dependency paths only. Nested maintained
# modules named vendor/target/etc. must still be inspected. The exact vendored
# Tao, GPUI Kit and AccessKit dependencies are third-party code, not Tauri
# transport owners. Authored GPUI sources remain scanned. .git is metadata.
EXCLUDED = {Path('.git'), Path('app/node_modules'), Path('node_modules'),
            Path('app/src-tauri/target'), Path('target'), Path('app/src-tauri/patches/tao'),
            Path('app/gpui/target'), Path('app/gpui/vendor/gpui-component'),
            Path('app/gpui/vendor/accesskit_macos'), Path('app/gpui/vendor/gpui')}


def excluded(path: Path, root: Path) -> bool:
    relative = path.relative_to(root)
    return any(relative == prefix or prefix in relative.parents for prefix in EXCLUDED)


IDENTIFIER = re.compile(r'(?:r#)?[^\W\d]\w*', re.UNICODE)
TRANSPORT = re.compile(r'emit(?:_\w*)?\Z', re.UNICODE)


def opaque_spans(source: str):
    """Rust comments/strings/chars, including nested comments and raw strings."""
    pattern = re.compile(r'//[^\n]*|/\*|(?:br|cr|r)(#+)?"|(?:b|c)?"|b?\'(?:\\.|[^\'\\\n])\'')
    position = 0
    while match := pattern.search(source, position):
        start, end = match.span()
        token = match.group()
        kind = 'literal'
        if token.startswith('//'):
            kind = 'comment'
        elif token == '/*':
            kind = 'comment'
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
        elif 'r' in token and '"' in token:
            terminator = '"' + (match.group(1) or '')
            close = source.find(terminator, end)
            end = len(source) if close < 0 else close + len(terminator)
        elif token.endswith('"'):
            while end < len(source):
                if source[end] == '\\':
                    end += 2
                elif source[end] == '"':
                    end += 1
                    break
                else:
                    end += 1
        yield kind, start, min(end, len(source))
        position = end


def mask_literals(source: str) -> str:
    chars = list(source)
    for _, start, end in opaque_spans(source):
        for i in range(start, end):
            if chars[i] != '\n':
                chars[i] = ' '
    return ''.join(chars)


def violations(source: str, *, adapter: bool = False, gpui_adapter: bool = False) -> list[int]:
    code = mask_literals(source)
    # Preserve offsets when normalizing raw identifiers.
    code = re.sub(r'\br#(?=[^\W\d])', '  ', code)
    offsets = set()
    for token in IDENTIFIER.finditer(code):
        name = token.group()
        if TRANSPORT.fullmatch(name):
            # This exact two-argument UFCS call can only publish a typed local
            # GPUI entity event. Never permit a method call or Tauri emitter
            # through the native adapter; it remains in the source inventory.
            local_gpui = (gpui_adapter and name == 'emit'
                          and re.search(r'\bgpui\s*::\s*Context\s*::\s*<\s*T\s*>\s*::\s*$', code[:token.start()])
                          and re.match(r'\s*\(\s*cx\s*,\s*event\s*\)', code[token.end():]))
            if local_gpui:
                continue
            if not adapter or name not in {'emit_to', 'emit_str_to'}:
                offsets.add(token.start())
            elif not re.match(
                r'\s*\(\s*tauri\s*::\s*EventTarget\s*::\s*webview_window\s*\(\s*label\s*\)\s*,',
                code[token.end():],
            ):
                offsets.add(token.start())
        if name == 'Emitter' and not adapter:
            offsets.add(token.start())
        if name == 'EventTarget' and adapter and not re.match(
            r'\s*::\s*webview_window\b', code[token.end():]
        ):
            offsets.add(token.start())
    if adapter:
        # The adapter has no macro definitions/forwarding. A target or method
        # must not be hidden behind a metavariable in this sanctioned file.
        offsets.update(m.start() for m in re.finditer(r'\$', code))
    # Keep the adapter from becoming a re-export facade itself. No maintained
    # source needs a tauri glob, crate alias or re-export, even in that file.
    for statement in re.finditer(r'\buse\b[^;]*;', code):
        body = statement.group()
        exported = re.search(r'\bpub(?:\s*\([^)]*\))?\s*$', code[:statement.start()])
        if adapter and exported:
            offsets.add(statement.start())
        if re.search(r'\btauri\b', body):
            if ('*' in body or re.search(r'\btauri\s*(?:as\b|[,;}])', body)
                    or re.search(r'\bself\s+as\b', body)
                    or exported):
                offsets.add(statement.start())
    offsets.update(m.start() for m in re.finditer(r'\bextern\s+crate\s+tauri\b', code))
    return sorted({code.count('\n', 0, offset) + 1 for offset in offsets})


def rust_sources(root: Path) -> list[Path]:
    paths = []
    for directory, dirs, files in os.walk(root, followlinks=False):
        dirs[:] = sorted(d for d in dirs if not excluded(Path(directory) / d, root))
        paths.extend(Path(directory) / name for name in sorted(files) if name.endswith('.rs'))
    return sorted(paths)


def rust_string(literal: str) -> str | None:
    if match := re.fullmatch(r'r(#+)?"(.*)"\1', literal, re.DOTALL):
        return match.group(2)
    if literal.startswith('r"') and literal.endswith('"'):
        return literal[2:-1]
    try:
        value = ast.literal_eval(literal)
        return value if isinstance(value, str) else None
    except (ValueError, SyntaxError):
        return None


def source_references(source: str, path: Path, scanned: set[Path]) -> list[int]:
    """Reject Rust inclusion whose literal destination is not scanned.

    Dynamic inclusion is rejected rather than pretending to resolve macro/env
    expansion. Non-Rust include_str!/include_bytes! fixtures are unaffected.
    """
    code = re.sub(r'\br#(?=[^\W\d])', '  ', mask_literals(source))
    literals = {start: end for kind, start, end in opaque_spans(source) if kind == 'literal'}
    failures = set()
    # Conditional path attributes and inline-module path bases are rejected
    # conservatively; the maintained paths use ordinary file-level attributes.
    for match in re.finditer(r'#\s*\[\s*cfg_attr\b[^;]*?\bpath\s*=', code):
        failures.add(match.start())
    # A macro cannot forward an uninspected module attribute or alias/forward
    # the built-in inclusion macro. Direct literal inclusions are checked below.
    failures.update(m.start() for m in re.finditer(r'#\s*\[[^\]]*\$', code))
    for token in IDENTIFIER.finditer(code):
        if token.group() in {'include', 'include_str', 'include_bytes'} and not re.match(
            r'\s*!\s*[({\[]', code[token.end():]
        ):
            failures.add(token.start())
    patterns = [r'#\s*\[\s*path\s*=', r'\binclude(?:_str|_bytes)?\s*!\s*[({\[]']
    for pattern in patterns:
        for match in re.finditer(pattern, code):
            # Comments can precede the literal. Use their masked offsets.
            starts = [start for start in literals if start >= match.end()]
            start = min(starts, default=len(source))
            if code[match.end():start].strip() or start not in literals:
                failures.add(match.start())
                continue
            literal_end = literals[start]
            value = rust_string(source[start:literal_end])
            # Only one literal argument is permitted, with optional trailing
            # comma for macros. This closes concat/env and token-suffix tricks.
            tail = code[literal_end:]
            is_path = pattern.startswith('#')
            if is_path and code[:match.start()].count('{') != code[:match.start()].count('}'):
                failures.add(match.start())
                continue
            if value is None or not re.match(r'\s*\]' if is_path else r'\s*,?\s*[)}\]]', tail):
                failures.add(match.start())
                continue
            if is_path or value.endswith('.rs') or re.match(r'\binclude\s*!', match.group()):
                if (path.parent / value).resolve() not in scanned:
                    failures.add(match.start())
    return sorted({code.count('\n', 0, offset) + 1 for offset in failures})


def cargo_roots(manifest: Path, root: Path, scanned: set[Path]) -> list[str]:
    """Cargo's explicit roots must not turn excluded output into source code.

    Resolve paths from each manifest (including .. and symlinks), not the repo
    root. Do not load dependencies or evaluate build scripts/Cargo metadata.
    """
    prefix = f'{manifest.relative_to(root)}:1: Cargo'
    resolved = manifest.resolve()
    if not resolved.is_relative_to(root) or excluded(resolved, root):
        return [f'{prefix} manifest outside scanned roots']
    try:
        data = tomllib.loads(manifest.read_text())
    except (OSError, ValueError) as error:
        return [f'{prefix} manifest cannot be parsed: {error}']
    failures = []

    def check(value, kind):
        if not isinstance(value, str) or (manifest.parent / value).resolve() not in scanned:
            failures.append(f'{prefix} {kind} root outside scanned Rust sources: {value!r}')

    for kind in ['lib', 'bin', 'test', 'example', 'bench']:
        tables = [data[kind]] if kind == 'lib' and kind in data else data.get(kind, [])
        if not isinstance(tables, list) or any(not isinstance(table, dict) for table in tables):
            failures.append(f'{prefix} invalid {kind} target declaration')
            continue
        for table in tables:
            if 'path' in table:
                check(table['path'], kind)
    package = data.get('package', {})
    if not isinstance(package, dict):
        return failures + [f'{prefix} invalid package declaration']
    build = package.get('build', True)
    if isinstance(build, str):
        check(build, 'build')
    elif build is True:
        default = manifest.parent / 'build.rs'
        if default.exists() or default.is_symlink():
            check('build.rs', 'build')
    elif build is not False:
        failures.append(f'{prefix} invalid build script declaration')
    return failures


def check_repository(root: Path) -> list[str]:
    root = root.resolve()
    paths = rust_sources(root)
    scanned = {path.resolve() for path in paths if path.resolve().is_relative_to(root)
               and not excluded(path.resolve(), root)}
    failures = []
    for directory, dirs, files in os.walk(root, followlinks=False):
        dirs[:] = [name for name in dirs if not excluded(Path(directory) / name, root)]
        if 'Cargo.toml' in files:
            failures.extend(cargo_roots(Path(directory) / 'Cargo.toml', root, scanned))
        for name in dirs:
            path = Path(directory) / name
            if path.is_symlink() and (not path.resolve().is_relative_to(root)
                    or excluded(path.resolve(), root)):
                failures.append(f'{path.relative_to(root)}:1: directory symlink outside scanned roots')
    for path in paths:
        relative = path.relative_to(root)
        if path.resolve() not in scanned:
            failures.append(f'{relative}:1: Rust source resolves outside scanned roots')
            continue
        source = path.read_text()
        failures.extend(f'{relative}:{line}: reserved transport token/import/target'
                        for line in violations(source, adapter=relative == ADAPTER,
                                               gpui_adapter=relative == GPUI_ADAPTER))
        failures.extend(f'{relative}:{line}: Rust inclusion outside scanned sources or nonliteral path'
                        for line in source_references(source, path, scanned))
    return failures


def main() -> int:
    failures = check_repository(ROOT)
    if failures:
        print('\n'.join(failures), file=sys.stderr)
        return 1
    print('repository Rust: Tauri emission confined to the constrained sink; typed GPUI entity adapter, source inclusions and Cargo roots checked')
    return 0


if __name__ == '__main__':
    sys.exit(main())
