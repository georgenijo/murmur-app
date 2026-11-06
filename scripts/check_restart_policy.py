#!/usr/bin/env python3
"""Rust-token policy: reserve process relaunch identifiers, including imports,
UFCS, aliases, raw identifiers, method references and macro token trees.
Uses the event guard's nested-comment/raw-string-aware Rust lexer and its
fail-closed repository/inclusion inventory. This deliberately reserves names
rather than attempting incomplete Rust type inference.
"""
from pathlib import Path
import re
import os
import tomllib
from check_targeted_events import ROOT, IDENTIFIER, mask_literals, rust_sources, excluded

ADAPTER = Path('app/src-tauri/src/shutdown.rs')
PLUGIN = Path('app/src-tauri/src/process_plugin.rs')

def violations(source, *, adapter=False, plugin=False, process_aliases=(), cli_entry=False):
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
            # Only ordinary zero-code runtime exit calls are admitted. Unknown
            # expressions (including const aliases, casts and macro arguments)
            # are refused rather than guessed safe. References/UFCS are refused.
            runtime_zero = re.search(r'\.\s*$', before) and re.match(r'\s*\(\s*0\s*\)', after)
            definition = plugin and re.search(r'\bfn\s*$', before)
            handler = plugin and re.search(r'tauri\s*::\s*generate_handler\s*!\s*\[\s*$', before)
            process_call = re.search(r'\bstd\s*::\s*process\s*::\s*$', before)
            # Existing CLI dispatch exits before Tauri is started. Keep its
            # exact local result spelling confined to that entry point.
            process_argument = re.match(r'\s*\(\s*(0|1|64|70|74|99)\s*\)', after)
            cli_result = cli_entry and re.match(r'\s*\(\s*exit_code\s*\)', after)
            exit_use = re.search(r'::\s*$', before) or re.match(r'\s*(?:\(|!)', after)
            if exit_use and not (runtime_zero or definition or handler or (process_call and (process_argument or cli_result))):
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
        for offset in violations(source, adapter=relative == ADAPTER, plugin=relative == PLUGIN,
                                 process_aliases=aliases,
                                 cli_entry=relative == Path('app/src-tauri/src/main.rs')):
            failures.append(f'{relative}:{source.count(chr(10), 0, offset)+1}: forbidden process restart bypass')
    return failures


def main():
    failures = check_repository(ROOT)
    for failure in failures: print(failure)
    if not failures: print('Restart policy: sole drained adapter call; Cargo aliases and exit arguments checked')
    return bool(failures)
if __name__ == '__main__': raise SystemExit(main())
