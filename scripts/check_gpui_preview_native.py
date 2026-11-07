#!/usr/bin/env python3
"""Opt-in native preview regression: own PID/windows only; no HID or settings.

Run on macOS with the GUI owner's agreement. Captures only windows belonging
to the freshly built synthetic host, never the desktop or another process.
The normal headless suite checks paint bounds without opening native windows.
"""
from pathlib import Path
import hashlib
import json
import os
import platform
import selectors
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
UI = ROOT / 'app/gpui'
OBSERVER = r'''
#import <AppKit/AppKit.h>
#import <Vision/Vision.h>
#import <ImageIO/ImageIO.h>
int main(int argc, const char **argv) {
    @autoreleasepool {
        if (argc != 3) return 2;
        if (!strcmp(argv[1], "window")) {
            pid_t pid = atoi(argv[2]);
            NSArray *windows = CFBridgingRelease(CGWindowListCopyWindowInfo(kCGWindowListOptionAll, kCGNullWindowID));
            for (NSDictionary *w in windows) {
                if ([w[(__bridge id)kCGWindowOwnerPID] intValue] != pid ||
                    [w[(__bridge id)kCGWindowAlpha] doubleValue] <= 0) continue;
                CGRect bounds;
                if (!CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)w[(__bridge id)kCGWindowBounds], &bounds) ||
                    bounds.size.width < 400 || bounds.size.height < 100) continue;
                printf("%u\n", [w[(__bridge id)kCGWindowNumber] unsignedIntValue]);
                return 0;
            }
            return 3;
        }
        if (strcmp(argv[1], "image")) return 2;
        NSURL *url = [NSURL fileURLWithPath:@(argv[2])];
        CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)url, NULL);
        CGImageRef image = source ? CGImageSourceCreateImageAtIndex(source, 0, NULL) : NULL;
        if (!image) return 4;
        VNRecognizeTextRequest *request = [VNRecognizeTextRequest new];
        request.recognitionLevel = VNRequestTextRecognitionLevelAccurate;
        request.usesLanguageCorrection = NO;
        NSError *error = nil;
        VNImageRequestHandler *handler = [[VNImageRequestHandler alloc] initWithCGImage:image options:@{}];
        BOOL ok = [handler performRequests:@[request] error:&error];
        NSMutableArray *words = [NSMutableArray new];
        for (VNRecognizedTextObservation *result in request.results) {
            VNRecognizedText *candidate = [result topCandidates:1].firstObject;
            if (candidate) [words addObject:candidate.string];
        }
        NSDictionary *result = @{@"width": @(CGImageGetWidth(image)), @"height": @(CGImageGetHeight(image)),
            @"text": [words componentsJoinedByString:@" "], @"ocrSucceeded": @(ok)};
        NSData *json = [NSJSONSerialization dataWithJSONObject:result options:0 error:NULL];
        puts([[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding].UTF8String);
        CGImageRelease(image);
        CFRelease(source);
        return ok ? 0 : 5;
    }
}
'''


def main():
    if platform.system() != 'Darwin':
        raise SystemExit('native preview regression requires macOS')
    subprocess.run(['cargo', 'build', '--locked', '--features', 'gallery', '--bin', 'preview-qa'],
                   cwd=UI, check=True)
    target = Path(json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--no-deps', '--format-version=1'], cwd=UI))['target_directory'])
    evidence = target / 'preview-native-regression'
    evidence.mkdir(parents=True, exist_ok=True)
    source = evidence / 'observer.m'
    source.write_text(OBSERVER)
    observer = evidence / 'observer'
    subprocess.run(['clang', '-fobjc-arc', '-Werror', '-framework', 'AppKit', '-framework', 'Vision',
                    '-framework', 'ImageIO', str(source), '-o', str(observer)], check=True)
    env = {k: v for k, v in os.environ.items() if not k.startswith('MURMUR_')}
    env['MURMUR_PREVIEW_NATIVE_SMOKE'] = '1'
    observations = []
    log = evidence / 'host.log'
    with log.open('w') as stderr:
        host = subprocess.Popen([str(target / 'debug/preview-qa')], cwd=UI, env=env,
                                stdout=subprocess.PIPE, stderr=stderr, text=True)
        selector = selectors.DefaultSelector()
        selector.register(host.stdout, selectors.EVENT_READ)
        deadline = time.monotonic() + 60
        try:
            while time.monotonic() < deadline:
                if not selector.select(timeout=1):
                    if host.poll() is not None:
                        break
                    continue
                line = host.stdout.readline()
                if not line:
                    break
                stderr.write(line)
                stderr.flush()
                if not line.startswith('MURMUR_PREVIEW_STAGE='):
                    continue
                stage = line.strip().split('=', 1)[1]
                observation = {'stage': stage, 'pid': host.pid}
                if stage.startswith('visible-'):
                    # Wait for the display-server transaction, without input.
                    time.sleep(0.6)
                    assert host.poll() is None, 'native host crashed before capture'
                    window = subprocess.check_output([str(observer), 'window', str(host.pid)], text=True).strip()
                    image = evidence / f'{stage}.png'
                    subprocess.run(['screencapture', '-x', '-o', '-l', window, str(image)], check=True)
                    result = json.loads(subprocess.check_output([str(observer), 'image', str(image)], text=True))
                    observation.update(window=int(window), image=image.name, **result)
                    assert 'synthetic native fixture' in result['text'].lower(), result
                observations.append(observation)
            assert host.wait(timeout=5) == 0, 'native smoke exited abnormally'
            expected = ['empty-1', 'stopped-1']
            for generation in range(2, 5):
                expected += [f'empty-{generation}', f'visible-{generation}', f'stopped-{generation}']
            assert [o['stage'] for o in observations] == expected, observations
            binary = target / 'debug/preview-qa'
            sources = ('app/gpui/src/bin/preview-qa.rs', 'app/gpui/src/preview/mod.rs', 'app/gpui/native/overlay.m')
            receipt = {'verdict': 'PASS', 'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                       'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                       'source_sha256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in sources},
                       'observations': observations}
            (evidence / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
            print(json.dumps(receipt, indent=2))
        finally:
            selector.close()
            if host.poll() is None:
                host.terminate()
                host.wait(timeout=5)


if __name__ == '__main__':
    main()
