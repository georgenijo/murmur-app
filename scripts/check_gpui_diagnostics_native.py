#!/usr/bin/env python3
"""Opt-in Mac GUI regression: standard synthetic launch, native focus/typing, WebKit.

Requires existing Accessibility permission; never requests a grant, changes
VoiceOver, reads production data, or uses the pasteboard. Run only while owning
the GUI. Pass a built diagnostics-qa binary and a new evidence directory.
"""
import argparse
import ctypes
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import time

from check_gpui_goldens import check_inventory

PROBE = r'''
import AppKit
import ApplicationServices
let pid = Int32(CommandLine.arguments[2])!
let app = NSRunningApplication(processIdentifier: pid)!
func attribute(_ e: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    return AXUIElementCopyAttributeValue(e, name as CFString, &value) == .success ? value : nil
}
func field(_ e: AXUIElement, _ depth: Int = 0) -> AXUIElement? {
    if attribute(e, "AXRole") as? String == "AXTextField",
       attribute(e, "AXTitle") as? String == "Filter by Recording ID" { return e }
    if depth > 16 { return nil }
    for child in attribute(e, depth == 0 ? "AXWindows" : "AXChildren") as? [AXUIElement] ?? [] {
        if let found = field(child, depth + 1) { return found }
    }
    return nil
}
if CommandLine.arguments[1] == "quit" {
    guard app.terminate() else { exit(3) }
} else {
    guard AXIsProcessTrusted() else { fputs("Existing Accessibility permission required\n", stderr); exit(4) }
    app.activate(options: [])
    usleep(200000)
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else { exit(5) }
    let root = AXUIElementCreateApplication(pid)
    AXUIElementSetAttributeValue(root, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
    var found: AXUIElement?
    for _ in 0..<20 {
        found = field(root)
        if found != nil { break }
        usleep(100000)
    }
    guard let input = found,
          let position = attribute(input, "AXPosition"), let size = attribute(input, "AXSize") else { exit(6) }
    var point = CGPoint.zero; var extent = CGSize.zero
    AXValueGetValue(position as! AXValue, .cgPoint, &point)
    AXValueGetValue(size as! AXValue, .cgSize, &extent)
    point.x += extent.width / 2; point.y += extent.height / 2
    for type in [CGEventType.mouseMoved, .leftMouseDown, .leftMouseUp] {
        let event = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: point, mouseButton: .left)!
        event.flags = []
        event.post(tap: .cghidEventTap)
        usleep(20000)
    }
    usleep(150000)
    // Key code 26 is the ordinary "7" key. No clipboard or shadow editor state.
    for down in [true, false] {
        let event = CGEvent(keyboardEventSource: nil, virtualKey: 26, keyDown: down)!
        event.flags = []
        event.post(tap: .cghidEventTap)
        usleep(20000)
    }
    usleep(200000)
    guard attribute(input, "AXValue") as? String == "7" else { exit(7) }
    print("PASS native correlation focus and typing; voiceOver=\(NSWorkspace.shared.isVoiceOverEnabled)")
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    if platform.system() != 'Darwin':
        parser.error('native check requires macOS')
    binary = args.binary.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    source = output / 'probe.swift'
    source.write_text(PROBE)
    probe = output / 'probe'
    subprocess.run(['swiftc', str(source), '-o', str(probe)], check=True)
    contents = output / 'Diagnostics QA.app/Contents'
    (contents / 'MacOS').mkdir(parents=True)
    executable = contents / 'MacOS/diagnostics-qa'
    shutil.copy2(binary, executable)
    (contents / 'Info.plist').write_bytes(plistlib.dumps({
        'CFBundleIdentifier': 'com.localdictation.diagnostics-native-check',
        'CFBundleName': 'Diagnostics QA', 'CFBundleExecutable': 'diagnostics-qa',
        'CFBundlePackageType': 'APPL', 'NSHighResolutionCapable': True,
    }))
    subprocess.run(['codesign', '--force', '--sign', '-', str(contents.parent)], check=True)
    # Deliberately omit native-smoke: this must exercise the standard host.
    env = {k: v for k, v in os.environ.items() if not k.startswith('MURMUR_')}
    env['MURMUR_DIAGNOSTICS_STATE'] = 'events'
    log_path = output / 'standard-launch.log'
    with log_path.open('w') as log:
        process = subprocess.Popen([str(executable)], stdout=log, stderr=subprocess.STDOUT, env=env)
        try:
            time.sleep(2)
            assert process.poll() is None, 'standard launch exited before text entry'
            path = ctypes.create_string_buffer(4096)
            assert ctypes.CDLL('/usr/lib/libproc.dylib').proc_pidpath(process.pid, path, len(path)) > 0
            assert Path(os.fsdecode(path.value)) == executable
            result = subprocess.run([str(probe), 'type', str(process.pid)],
                                    text=True, capture_output=True, check=True)
            (output / 'typing.txt').write_text(result.stdout)
        finally:
            if process.poll() is None:
                subprocess.run([str(probe), 'quit', str(process.pid)], check=True)
                process.wait(timeout=10)
    assert process.returncode == 0, f'native host exited {process.returncode}'
    inventory = check_inventory(log_path.read_text(), 'MURMUR_DIAGNOSTICS_INVENTORY')
    (output / 'inventory.json').write_text(json.dumps(inventory, indent=2) + '\n')
    print('PASS standard native launch, focus and typing: zero unattributed WebKit images')


if __name__ == '__main__':
    main()
