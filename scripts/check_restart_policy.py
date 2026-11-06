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
from check_targeted_events import ROOT, IDENTIFIER, mask_literals, rust_sources, excluded

ADAPTER = Path('app/src-tauri/src/shutdown.rs')
PLUGIN = Path('app/src-tauri/src/process_plugin.rs')
# Exact function scopes only. No crate/module/impl/block-level lint exceptions.
METHOD_ALLOWLIST = {
    ('app/src-tauri/src/lib.rs', 'generated_context'),
    ('app/src-tauri/src/main.rs', 'main'),
    ('app/src-tauri/examples/murmur-eval.rs', 'main'),
    ('app/src-tauri/tests/shutdown_appkit.rs', 'main'),
    ('app/src-tauri/src/shutdown.rs', 'terminal'),
    ('app/src-tauri/src/shutdown.rs', 'test_support::shutdown_test_action'),
    ('app/src-tauri/src/coreml_installer.rs', 'start_parent_watchdog'),
    ('app/src-tauri/src/meeting_diarization.rs', 'run_cli_if_requested'),
    ('app/src-tauri/sidecars/capture/src/main.rs', 'main'),
    ('app/src-tauri/sidecars/capture/src/main.rs', 'supported::run'),
    ('app/src-tauri/sidecars/capture/src/production.rs', 'spawn_automatic_probe_watchdog'),
    ('app/src-tauri/sidecars/capture/src/production.rs', 'spawn_parent_watchdog'),
    ('app/src-tauri/sidecars/capture/src/production.rs', 'run'),
}
REQUIRED_METHODS = {
    'std::process::exit', 'std::process::abort', 'libc::exit', 'libc::_exit',
    'libc::abort', 'std::os::unix::process::CommandExt::exec',
    'tauri::AppHandle::exit', 'tauri::AppHandle::restart', 'tauri::AppHandle::request_restart',
    'tauri::process::restart', 'tauri_plugin_process::init',
    'tauri_plugin_process::commands::exit', 'tauri_plugin_process::commands::restart',
    'tauri_plugin_process::exit', 'tauri_plugin_process::restart',
}

def method_allow_scopes(source, relative):
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
        if not valid or not function or (str(relative), qualified) not in METHOD_ALLOWLIST:
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

def violations(source, *, adapter=False, plugin=False, process_aliases=(), cli_entry=False, method_scopes=()):
    code = re.sub(r'\br#(?=[^\W\d])', '  ', mask_literals(source))
    failures = []
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
    if adapter and len(list(re.finditer(r'\brequest_restart\b', code))) != 1:
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
                                 cli_entry=relative == Path('app/src-tauri/src/main.rs'), method_scopes=scopes):
            failures.append(f'{relative}:{source.count(chr(10), 0, offset)+1}: forbidden process restart bypass')
    return failures


def main():
    failures = check_repository(ROOT) + check_clippy_config(ROOT)
    for failure in failures: print(failure)
    if not failures: print('Restart policy: resolved clippy methods, scoped exceptions, Cargo aliases and exit arguments checked')
    return bool(failures)
if __name__ == '__main__': raise SystemExit(main())
