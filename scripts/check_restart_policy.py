#!/usr/bin/env python3
"""Rust-token policy: reserve process relaunch identifiers, including imports,
UFCS, aliases, raw identifiers, method references and macro token trees.
Uses the event guard's nested-comment/raw-string-aware Rust lexer and its
fail-closed repository/inclusion inventory. This deliberately reserves names
rather than attempting incomplete Rust type inference.
"""
from pathlib import Path
import re
from check_targeted_events import ROOT, IDENTIFIER, mask_literals, rust_sources

ADAPTER = Path('app/src-tauri/src/shutdown.rs')
PLUGIN = Path('app/src-tauri/src/process_plugin.rs')

def violations(source, *, adapter=False, plugin=False):
    code = re.sub(r'\br#(?=[^\W\d])', '  ', mask_literals(source))
    failures = []
    for token in IDENTIFIER.finditer(code):
        name = token.group()
        if name == 'request_restart':
            if not adapter or not re.search(r'\bapp\s*\.\s*$', code[:token.start()]) or not re.match(r'\s*\(\s*\)', code[token.end():]):
                failures.append(token.start())
        if name == 'tauri_plugin_process' or (name == 'RESTART_EXIT_CODE' and not adapter):
            failures.append(token.start())
        if name == 'restart':
            definition = re.search(r'\bfn\s*$', code[:token.start()])
            handler = re.search(r'tauri\s*::\s*generate_handler\s*!\s*\[\s*exit\s*,\s*$', code[:token.start()])
            if not plugin or not (definition or handler):
                failures.append(token.start())
    if adapter and len(list(re.finditer(r'\brequest_restart\b', code))) != 1:
        failures.append(0)
    return failures

def main():
    failures = []
    for path in rust_sources(ROOT):
        relative = path.relative_to(ROOT)
        source = path.read_text()
        for offset in violations(source, adapter=relative == ADAPTER, plugin=relative == PLUGIN):
            failures.append(f'{relative}:{source.count(chr(10), 0, offset)+1}: forbidden process restart bypass')
    for failure in failures: print(failure)
    if not failures: print('Restart policy: sole drained adapter call; no bypass identifiers')
    return bool(failures)
if __name__ == '__main__': raise SystemExit(main())
