#!/usr/bin/env python3
"""Opt-in native AX check of task-owned synthetic Home/onboarding processes.

No HID, focus/activation request, VoiceOver, pasteboard, TCC prompt or user data.
Requires existing Accessibility trust. Only exact subprocess PIDs are traversed
and terminated. Run after building the gallery binaries on the Mac mini.
"""
from pathlib import Path
import json
import os
import platform
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
OBSERVER = r'''
#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
static id attr(AXUIElementRef e, CFStringRef name) {
    CFTypeRef v = NULL;
    if (AXUIElementCopyAttributeValue(e,name,&v) != kAXErrorSuccess) return nil;
    return CFBridgingRelease(v);
}
static AXUIElementRef find(AXUIElementRef e, NSString *label, unsigned depth) {
    if (depth > 40) return NULL;
    for (NSString *key in @[@"AXTitle",@"AXDescription",@"AXValue"]) {
        id value=attr(e,(__bridge CFStringRef)key);
        if ([value isKindOfClass:NSString.class] && [value isEqualToString:label]) {
            CFRetain(e); return e;
        }
    }
    id children=attr(e,kAXChildrenAttribute);
    if (![children isKindOfClass:NSArray.class]) return NULL;
    for (id child in children) {
        AXUIElementRef result=find((__bridge AXUIElementRef)child,label,depth+1);
        if (result) return result;
    }
    return NULL;
}
int main(int argc,const char **argv) { @autoreleasepool {
    if (argc != 4) return 2;
    if (!strcmp(argv[2],"window")) {
        pid_t wanted=atoi(argv[1]);
        NSArray *windows=CFBridgingRelease(CGWindowListCopyWindowInfo(kCGWindowListOptionAll,kCGNullWindowID));
        for (NSDictionary *w in windows) {
            if ([w[(__bridge id)kCGWindowOwnerPID] intValue]!=wanted) continue;
            CGRect bounds;
            if (!CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)w[(__bridge id)kCGWindowBounds],&bounds) || bounds.size.width<400) continue;
            printf("%u\n",[w[(__bridge id)kCGWindowNumber] unsignedIntValue]);return 0;
        }
        return 3;
    }
    if (!AXIsProcessTrusted()) { fputs("Accessibility trust unavailable; no prompt requested\n",stderr); return 77; }
    pid_t pid=atoi(argv[1]); if (pid<=1) return 2;
    AXUIElementRef app=AXUIElementCreateApplication(pid);
    AXUIElementSetMessagingTimeout(app,3);
    if (!strcmp(argv[2],"close")) {
        NSArray *windows=attr(app,kAXWindowsAttribute);
        AXUIElementRef window=windows.count?(__bridge AXUIElementRef)windows.firstObject:NULL;
        id button=window?attr(window,kAXCloseButtonAttribute):nil;
        int result=button && AXUIElementPerformAction((__bridge AXUIElementRef)button,kAXPressAction)==kAXErrorSuccess?0:4;
        CFRelease(app);return result;
    }
    AXUIElementRef node=find(app,@(argv[3]),0);
    if (!node) {CFRelease(app);return 3;}
    int result=0;
    if (!strcmp(argv[2],"press")) result=AXUIElementPerformAction(node,kAXPressAction)==kAXErrorSuccess?0:4;
    printf("pid=%d action=%s label=%s result=%d\n",pid,argv[2],argv[3],result);
    CFRelease(node);CFRelease(app);return result;
}}
'''


def main():
    if platform.system() != 'Darwin':
        raise SystemExit('native AX verification requires macOS')
    ui = ROOT / 'app/gpui'
    target = Path(json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--no-deps', '--format-version=1'], cwd=ui))['target_directory'])
    evidence = target / 'onboarding-home-native'
    evidence.mkdir(parents=True, exist_ok=True)
    source = evidence / 'observer.m'
    source.write_text(OBSERVER)
    observer = evidence / 'observer'
    subprocess.run(['clang', '-fobjc-arc', '-Werror', '-framework', 'AppKit',
                    '-framework', 'ApplicationServices', str(source), '-o', str(observer)], check=True)
    observations = []
    env = {k: v for k, v in os.environ.items() if not k.startswith('MURMUR_')}
    flows = {
        'onboarding': [('find', 'Welcome to Murmur'), ('press', 'Get started'),
                       ('find', 'Microphone access'), ('press', 'Continue'),
                       ('find', 'Accessibility access'), ('press', 'Skip for now'),
                       ('find', 'System audio access'), ('press', 'Skip meetings for now'),
                       ('find', 'Calendar access'), ('press', 'Skip Calendar for now'),
                       ('find', 'Transcription model'), ('press', 'Continue'),
                       ('find', 'Recording shortcut'), ('press', 'Continue'),
                       ('find', "You're all set"), ('press', 'Start using Murmur'),
                       ('find', 'Setup complete')],
        'home': [('find', 'Dictation controls'), ('press', 'Click to start talking'),
                 ('find', 'Click to stop recording, 0:00'), ('press', 'Click to stop recording, 0:00'),
                 ('find', 'Writing it out…'), ('press', 'Collapse sidebar'),
                 ('find', 'Notetaker'), ('press', 'Expand sidebar'),
                 ('press', 'Dismiss Get more from Murmur')],
    }
    try:
        for family, steps in flows.items():
            with (evidence / f'{family}-host.log').open('w') as log:
                host = subprocess.Popen([str(target / f'debug/{family}-qa')], cwd=ui, env=env,
                                        stdout=log, stderr=log)
                try:
                    for action, label in steps:
                        deadline = time.monotonic() + 15
                        while True:
                            if host.poll() is not None:
                                raise RuntimeError(f'{family} test process exited')
                            result = subprocess.run([str(observer), str(host.pid), action, label],
                                                    capture_output=True, text=True, timeout=8)
                            if result.returncode == 0:
                                observations.append(dict(family=family, pid=host.pid, action=action, label=label))
                                break
                            if result.returncode == 77:
                                raise RuntimeError(result.stderr.strip())
                            if time.monotonic() >= deadline:
                                raise RuntimeError(f'{family}: AX {action} {label}: {result.returncode} {result.stderr}')
                            time.sleep(0.1)
                        if action == 'find':
                            window = subprocess.run([str(observer), str(host.pid), 'window', '-'],
                                                    capture_output=True, text=True, timeout=5)
                            if window.returncode == 0:
                                image = evidence / f'{family}-{len(observations):02}.png'
                                subprocess.run(['screencapture', '-x', '-o', '-l', window.stdout.strip(), str(image)],
                                               check=True, timeout=10)
                        time.sleep(0.15)
                    subprocess.run([str(observer), str(host.pid), 'close', '-'], check=True, timeout=8)
                    host.wait(timeout=15)
                    if host.returncode != 0:
                        raise RuntimeError(f'{family}: window close did not exit cleanly: {host.returncode}')
                    observations.append(dict(family=family, pid=host.pid, action='close', label='own test window'))
                finally:
                    if host.poll() is None:
                        host.terminate()
                        host.wait(timeout=15)
    finally:
        (evidence / 'receipt.json').write_text(json.dumps({'observations': observations,
            'transport': 'native AX API; no focus/HID/VoiceOver'}, indent=2) + '\n')
    print(f'PASS {len(observations)} native AX assertions/actions against exact synthetic PIDs')


if __name__ == '__main__':
    main()
