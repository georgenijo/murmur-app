#!/usr/bin/env python3
"""PID-only Apple-event sender for the isolated shutdown QA bundle.
No loginwindow/system event targets, bundle-ID addressing, or GUI input.
"""
import argparse
import ctypes
import os
from pathlib import Path
import plistlib
import subprocess
import sys

IDENTIFIER = 'com.localdictation.shutdown-test'
REASONS = {'quit': None, 'logout': 'rlgo', 'restart': 'rest', 'shutdown': 'shut'}

def validate_identity(bundle, pid, actual_executable):
    bundle = bundle.resolve()
    if pid <= 1 or bundle == Path('/Applications') or Path('/Applications') in bundle.parents:
        raise ValueError('Isolated build and explicit worker PID required')
    with (bundle / 'Contents/Info.plist').open('rb') as file:
        info = plistlib.load(file)
    if info.get('CFBundleIdentifier') != IDENTIFIER:
        raise ValueError('Wrong bundle identity')
    expected = (bundle / 'Contents/MacOS' / info['CFBundleExecutable']).resolve()
    if Path(actual_executable).resolve() != expected:
        raise ValueError('PID executable does not match the isolated bundle')
    return expected

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', required=True, type=Path)
    parser.add_argument('--pid', required=True, type=int)
    parser.add_argument('--reason', choices=REASONS, default='quit')
    args = parser.parse_args()
    if sys.platform != 'darwin': parser.error('macOS only')
    buffer = ctypes.create_string_buffer(4096)
    libproc = ctypes.CDLL('/usr/lib/libproc.dylib')
    if libproc.proc_pidpath(args.pid, buffer, len(buffer)) <= 0:
        parser.error('Verified isolated PID unavailable')
    expected = validate_identity(args.bundle, args.pid, os.fsdecode(buffer.value))
    reason = REASONS[args.reason]
    code = int.from_bytes(reason.encode('ascii'), 'big') if reason else 0
    # The SDK constants are kAEReallyLogOut='rlgo', kAERestart='rest',
    # kAEShutDown='shut', keyAEQuitReason='why?'. Never address loginwindow.
    script = '''import Foundation
let target = NSAppleEventDescriptor(processIdentifier: Int32(CommandLine.arguments[1])!)
let event = NSAppleEventDescriptor(eventClass: 0x61657674, eventID: 0x71756974, targetDescriptor: target, returnID: -1, transactionID: 0)
let reason = UInt32(CommandLine.arguments[2])!
if reason != 0 { event.setParam(NSAppleEventDescriptor(enumCode: reason), forKeyword: 0x7768793f) }
_ = try event.sendEvent(options: [.noReply, .neverInteract], timeout: 10)
'''
    # Revalidate immediately before the send, including PID reuse.
    buffer.value = b''
    if libproc.proc_pidpath(args.pid, buffer, len(buffer)) <= 0 or Path(os.fsdecode(buffer.value)).resolve() != expected:
        parser.error('Isolated process identity changed')
    subprocess.run(['swift', '-', str(args.pid), str(code)], input=script, text=True, check=True)
    print('Isolated PID-targeted quit event sent')

if __name__ == '__main__': main()
