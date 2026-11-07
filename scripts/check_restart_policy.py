#!/usr/bin/env python3
"""Rust-token policy: reserve process relaunch identifiers, including imports,
UFCS, aliases, raw identifiers, method references and macro token trees.
Uses the event guard's nested-comment/raw-string-aware Rust lexer and its
fail-closed repository/inclusion inventory. This deliberately reserves names
for relaunch APIs. Clippy resolves terminating methods, imports and macro forwarding.
"""
from pathlib import Path
import re
import os
import tomllib
from check_targeted_events import ROOT, IDENTIFIER, mask_literals, rust_sources, excluded, opaque_spans, rust_string

ADAPTER = Path('app/crates/murmur-services/src/shutdown.rs')
PLUGIN = Path('app/crates/murmur-services/src/process_plugin.rs')
# Exact function scopes only. No crate/module/impl/block-level lint exceptions.
METHOD_ALLOWLIST = {
    ('app/crates/murmur-services/src/lib.rs', 'generated_context'),
    ('app/crates/murmur-services/src/main.rs', 'main'),
    ('app/crates/murmur-services/examples/murmur-eval.rs', 'main'),
    ('app/crates/murmur-services/tests/shutdown_appkit.rs', 'main'),
    ('app/crates/murmur-services/src/shutdown.rs', 'terminal'),
    ('app/crates/murmur-services/src/shutdown.rs', 'test_support::shutdown_test_action'),
    ('app/crates/murmur-services/src/coreml_installer.rs', 'start_parent_watchdog'),
    ('app/crates/murmur-services/src/meeting_diarization.rs', 'run_cli_if_requested'),
    ('app/sidecars/capture/src/main.rs', 'main'),
    ('app/sidecars/capture/src/main.rs', 'supported::run'),
    ('app/sidecars/capture/src/production.rs', 'spawn_automatic_probe_watchdog'),
    ('app/sidecars/capture/src/production.rs', 'spawn_parent_watchdog'),
    ('app/sidecars/capture/src/production.rs', 'run'),
}
REQUIRED_METHODS = {
    'std::process::exit', 'std::process::abort', 'libc::exit', 'libc::_exit',
    'libc::abort', 'std::os::unix::process::CommandExt::exec',

}

def method_allow_scopes(source, relative, *, allowlist=METHOD_ALLOWLIST):
    code = mask_literals(source)
    scopes, failures = [], []
    for attribute in re.finditer(r'#!?\s*\[[^\]]*\]', code):
        value = attribute.group()
        if not re.search(r'\b(?:allow|expect)\s*\(', value):
            continue
        if not re.search(r'\b(?:disallowed_methods|disallowed_macros|warnings|restriction|all)\b|\(\s*clippy\s*[,)]', value):
            continue
        function = re.match(r'\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)\b[^;{]*\{', code[attribute.end():])
        valid = re.fullmatch(r'#\s*\[\s*allow\s*\(\s*clippy\s*::\s*disallowed_methods\s*\)\s*\]', value)
        enclosing = []
        for brace in re.finditer(r'[{}]', code[:attribute.start()]):
            if brace.group() == '}':
                if enclosing:
                    enclosing.pop()
            else:
                module = re.search(r'\bmod\s+(\w+)\s*$', code[:brace.start()])
                enclosing.append(module[1] if module else None)
        qualified = '::'.join([*enclosing, function[1]]) if function and None not in enclosing else None
        if not valid or not function or (str(relative), qualified) not in allowlist:
            failures.append(attribute.start())
            continue
        opening = attribute.end() + function.end() - 1
        depth, end = 1, opening + 1
        while end < len(code) and depth:
            depth += (code[end] == '{') - (code[end] == '}')
            end += 1
        if depth:
            failures.append(attribute.start())
        else:
            scopes.append((opening, end))
    return scopes, failures

def check_clippy_config(root):
    try:
        data = tomllib.loads((root / 'clippy.toml').read_text())
        paths = {entry if isinstance(entry, str) else entry['path'] for entry in data['disallowed-methods']}
        if not REQUIRED_METHODS.issubset(paths):
            return ['clippy.toml: terminating method policy is incomplete']
    except (OSError, ValueError, KeyError, TypeError):
        return ['clippy.toml: missing or invalid terminating method policy']
    failures = []
    for directory, dirs, files in os.walk(root, followlinks=False):
        dirs[:] = sorted(d for d in dirs if not excluded(Path(directory) / d, root))
        for name in {'clippy.toml', '.clippy.toml'}.intersection(files):
            path = Path(directory) / name
            if path != root / 'clippy.toml':
                failures.append(f'{path.relative_to(root)}: nested clippy configuration can override the process guard')
    return failures

TERMINATING_SYMBOLS = {'exit', '_exit', '_Exit', 'abort', 'quick_exit', 'kill',
                       'raise', 'pthread_exit', 'exit_group'}

def symbol_literal_allowances(source, relative):
    # The isolated QA handler uses "exit" as an action discriminator. Permit
    # only its exact match arm in that function, never arbitrary literals in
    # an otherwise sanctioned function or a dynamic lookup in this handler.
    if relative != ADAPTER:
        return set()
    scopes, _ = method_allow_scopes(source, relative, allowlist={
        (str(ADAPTER), 'test_support::shutdown_test_action'),
    })
    return {
        arm.start(1)
        for arm in re.finditer(r'\bmatch\s+action\.as_str\(\)\s*\{\s*("exit")\s*=>\s*app\.exit\(0\),', source)
        if any(start <= arm.start() < end for start, end in scopes)
    }


def delimiter_end(code, opening):
    closing = {'{': '}', '(': ')', '[': ']'}[code[opening]]
    depth, end = 1, opening + 1
    while end < len(code) and depth:
        depth += (code[end] == code[opening]) - (code[end] == closing)
        end += 1
    return end


def symbol_string(literal):
    # Byte/C prefixes do not affect the ASCII symbols reserved here.
    if literal.startswith(('b"', 'c"', 'br"', 'cr"', 'br#', 'cr#')):
        literal = literal[1:]
    if literal.startswith('r'):
        return rust_string(literal)
    if not (literal.startswith('"') and literal.endswith('"')):
        return None

    def unescape(match):
        escape = match.group()[1:]
        if escape.startswith('x'):
            return chr(int(escape[1:], 16))
        if escape.startswith('u{'):
            return chr(int(escape[2:-1].replace('_', ''), 16))
        if escape.startswith(('\n', '\r\n')):
            return ''
        return {'0': '\0', 'n': '\n', 'r': '\r', 't': '\t',
                '\\': '\\', '"': '"', "'": "'"}.get(escape, match.group())

    try:
        return re.sub(r'\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F_]+\}|\r?\n\s*|.)', unescape, literal[1:-1])
    except ValueError:
        return None


def foreign_termination(source, *, symbol_allowances=()):
    """Clippy cannot resolve a locally declared foreign alias to libc.

    Inspect foreign names with *any* ABI, macro-defined extern blocks, linker
    attributes and exact symbol literals used by dynamic lookups. Comments and
    non-symbol string contents remain opaque. No function-scope exception
    permits this. Macro-defined extern blocks fail closed without expansion.
    """
    code = re.sub(r'\br#(?=[^\W\d])', '  ', mask_literals(source))
    failures = []
    macro_scopes = [
        (macro.start(), delimiter_end(code, macro.end() - 1))
        for macro in re.finditer(r'\bmacro_rules\s*!\s*\w+\s*[({\[]', code)
    ]
    for start, end in macro_scopes:
        # Reject the declaration rather than trying to expand arbitrary macro
        # arguments, including ABI repetitions such as extern $($abi)* { ... }.
        failures.extend(start + token.start() for token in re.finditer(r'\bextern\b', code[start:end]))
    for block in re.finditer(r'\bextern\s*(?:\$\s*\w+\s*)?\{', code):
        opening = block.end() - 1
        end = delimiter_end(code, opening)
        if '$' in block.group():
            failures.append(block.start())
        for declaration in re.finditer(r'\bfn\s+(\$\s*)?(\w+)\b', code[opening:end]):
            if declaration[1] or declaration[2] in TERMINATING_SYMBOLS:
                failures.append(opening + declaration.start())
    literals = [(start, end) for kind, start, end in opaque_spans(source) if kind == 'literal']
    for start, end in literals:
        value = symbol_string(source[start:end])
        if (value is not None and value.removesuffix('\0') in TERMINATING_SYMBOLS
                and start not in symbol_allowances):
            failures.append(start)
    for attribute in re.finditer(r'#\s*\[[^\]]*\]', code):
        body = attribute.group()
        # Includes nested cfg_attr(..., link_name=...) and link(...).
        if not re.search(r'\b(?:link_name\s*=|link\s*\()', body):
            continue
        for name in re.finditer(r'\blink_name\s*=', body):
            value_start = attribute.start() + name.end()
            literal = next(((start, end) for start, end in literals if start >= value_start), None)
            if (literal is None or literal[0] >= attribute.end()
                    or code[value_start:literal[0]].strip()
                    or not re.match(r'\s*[,\])]', code[literal[1]:])):
                failures.append(value_start)
        if '!' in body or '$' in body:
            failures.append(attribute.start())
        for start, end in literals:
            if attribute.start() <= start < attribute.end():
                value = rust_string(source[start:end])
                # Unparsed linker syntax fails closed rather than blessing a
                # potentially escaped terminating symbol.
                if value is None or '\\' in value or value in TERMINATING_SYMBOLS:
                    failures.append(start)
    return failures


def violations(source, *, adapter=False, plugin=False, process_aliases=(), cli_entry=False, method_scopes=(), symbol_allowances=()):
    code = re.sub(r'\br#(?=[^\W\d])', '  ', mask_literals(source))
    failures = foreign_termination(source, symbol_allowances=symbol_allowances)
    for token in IDENTIFIER.finditer(code):
        name = token.group()
        if name == 'request_restart':
            if not adapter or not re.search(r'\bapp\s*\.\s*$', code[:token.start()]) or not re.match(r'\s*\(\s*\)', code[token.end():]):
                failures.append(token.start())
        if name in {'tauri_plugin_process', *process_aliases} or (name == 'RESTART_EXIT_CODE' and not adapter):
            failures.append(token.start())
        if name == 'exit':
            before, after = code[:token.start()], code[token.end():]
            # Clippy resolves aliases, grouped imports and macro token forwarding.
            # Keep the conservative numeric runtime-code check, but stop reserving
            # unrelated bare functions named exit.
            method = re.search(r'\.\s*$', before) and re.match(r'\s*\(', after)
            process_call = re.search(r'\b(?:std\s*::\s*process|libc)\s*::\s*$', before)
            runtime_zero = method and re.match(r'\s*\(\s*0\s*\)', after)
            sanctioned = any(start <= token.start() < end for start, end in method_scopes)
            process_argument = re.match(r'\s*\(\s*(0|1|64|70|74|99)\s*\)', after)
            cli_result = cli_entry and re.match(r'\s*\(\s*exit_code\s*\)', after)
            if method and not runtime_zero:
                failures.append(token.start())
            elif process_call and not ((cli_entry or sanctioned) and (process_argument or cli_result)):
                failures.append(token.start())
            elif re.search(r'\b(?:AppHandle|App)\s*::\s*$', before):
                failures.append(token.start())
        if name == 'restart':
            definition = re.search(r'\bfn\s*$', code[:token.start()])
            handler = re.search(r'tauri\s*::\s*generate_handler\s*!\s*\[\s*exit\s*,\s*$', code[:token.start()])
            if not plugin or not (definition or handler):
                failures.append(token.start())
    if adapter and re.search(r'\brequest_restart\b', code):
        failures.append(0)
    return failures

def process_dependency_aliases(root):
    """Inspect every maintained manifest, including workspace, target, patch,
    and replace tables. Reserve renamed package identifiers across the tree so
    workspace inheritance and Rust import aliases cannot hide the plugin.
    No Cargo execution, dependency loading, or generated code is trusted here.
    """
    aliases = {'tauri_plugin_process'}
    failures = []
    for directory, dirs, files in os.walk(root, followlinks=False):
        dirs[:] = sorted(d for d in dirs if not excluded(Path(directory) / d, root))
        if 'Cargo.toml' not in files:
            continue
        manifest = Path(directory) / 'Cargo.toml'
        if not manifest.resolve().is_relative_to(root.resolve()):
            failures.append(f'{manifest.relative_to(root)}:1: Cargo manifest outside scanned roots')
            continue
        try:
            data = tomllib.loads(manifest.read_text())
        except (OSError, ValueError) as error:
            failures.append(f'{manifest.relative_to(root)}:1: cannot parse Cargo manifest: {error}')
            continue

        def visit(table):
            for name, value in table.items():
                if name.split(':')[0] == 'tauri-plugin-process' or (
                    isinstance(value, dict) and value.get('package') == 'tauri-plugin-process'
                ):
                    aliases.add(name.split(':')[0].replace('-', '_'))
                if isinstance(value, dict):
                    visit(value)
        visit(data)
        if data.get('package', {}).get('name') == 'tauri-plugin-process':
            aliases.add(data.get('lib', {}).get('name', 'tauri_plugin_process'))
    return aliases, failures


def check_repository(root):
    aliases, failures = process_dependency_aliases(root)
    for path in rust_sources(root):
        relative = path.relative_to(root)
        source = path.read_text()
        scopes, invalid_allows = method_allow_scopes(source, relative)
        for offset in invalid_allows:
            failures.append(f'{relative}:{source.count(chr(10), 0, offset)+1}: forbidden process lint allowance')
        for offset in violations(source, adapter=relative == ADAPTER, plugin=relative == PLUGIN,
                                 process_aliases=aliases,
                                 cli_entry=relative == Path('app/crates/murmur-services/src/main.rs'), method_scopes=scopes,
                                 symbol_allowances=symbol_literal_allowances(source, relative)):
            failures.append(f'{relative}:{source.count(chr(10), 0, offset)+1}: forbidden process restart bypass')
    return failures


def main():
    failures = check_repository(ROOT) + check_clippy_config(ROOT)
    for failure in failures: print(failure)
    if not failures: print('Restart policy: resolved clippy methods, scoped exceptions, Cargo aliases, exit arguments, foreign declarations and symbol literals checked')
    return bool(failures)
if __name__ == '__main__': raise SystemExit(main())
