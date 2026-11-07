"""Compile the focus shim against inert classes; never create AppKit windows."""
from pathlib import Path
import platform
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(platform.system() == 'Darwin', 'Objective-C runtime requires macOS')
class FocusHookTests(unittest.TestCase):
    def test_effective_appearance_selects_light_dark_and_accessible_variants(self):
        source = r'''
#import <AppKit/AppKit.h>
#include <assert.h>
extern unsigned murmur_display_preferences(void);
int main(void) {
    @autoreleasepool {
        NSApplication *app = NSApplication.sharedApplication;
        unsigned preferences = murmur_display_preferences() & 7;
        for (NSString *name in @[NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
                NSAppearanceNameAccessibilityHighContrastAqua,
                NSAppearanceNameAccessibilityHighContrastDarkAqua, NSAppearanceNameAqua]) {
            // Process-local appearance only: no windows, activation, or settings changes.
            app.appearance = [NSAppearance appearanceNamed:name];
            unsigned bits = murmur_display_preferences();
            assert((bits & 7) == preferences);
            assert(((bits & 8) != 0) == [name containsString:@"Dark"]);
        }
    }
}
'''
        with tempfile.TemporaryDirectory(prefix='murmur-appearance-') as directory:
            path = Path(directory)
            (path / 'test.m').write_text(source)
            subprocess.run(['clang', '-fobjc-arc', '-Werror', '-framework', 'AppKit',
                            str(ROOT / 'app/gpui/native/preferences.m'),
                            str(path / 'test.m'), '-o', str(path / 'test')], check=True)
            subprocess.run([str(path / 'test')], check=True)

    def test_window_and_panel_forward_focus_and_keep_fallbacks(self):
        source = r'''
#import <Foundation/Foundation.h>
#import <objc/runtime.h>
#include <assert.h>
@interface Content : NSObject
@end
@implementation Content
- (id)accessibilityFocusedUIElement { return self; }
@end
@interface OriginalWindow : NSObject
@property id contentView;
@end
@implementation OriginalWindow
- (NSArray *)accessibilityAttributeNames { return @[@"AXTitle"]; }
- (id)accessibilityAttributeValue:(NSString *)attribute {
    return [attribute isEqualToString:@"AXTitle"] ? @"synthetic window" : nil;
}
@end
@interface GPUIWindow : OriginalWindow
@end
@implementation GPUIWindow
@end
@interface GPUIPanel : OriginalWindow
@end
@implementation GPUIPanel
@end
@interface OriginalApplication : NSObject
@property id keyWindow;
@end
@implementation OriginalApplication
- (id)accessibilityFocusedUIElement { return self; }
@end
@interface GPUIApplication : OriginalApplication
@end
@implementation GPUIApplication
@end
extern void murmur_install_focus_hook(void);
int main(void) {
    @autoreleasepool {
        murmur_install_focus_hook();
        murmur_install_focus_hook();
        id content = [Content new];
        for (Class cls in @[[GPUIWindow class], [GPUIPanel class]]) {
            id window = [cls new];
            assert([window respondsToSelector:@selector(accessibilityFocusedUIElement)]);
            assert([window accessibilityFocusedUIElement] == window);
            assert([[window accessibilityAttributeNames] containsObject:@"AXFocusedUIElement"]);
            assert([[window accessibilityAttributeNames] containsObject:@"AXTitle"]);
            assert([[window accessibilityAttributeValue:@"AXTitle"] isEqualToString:@"synthetic window"]);
            assert([window accessibilityAttributeValue:@"AXFocusedUIElement"] == window);
            [window setContentView:content];
            assert([window accessibilityFocusedUIElement] == content);
            assert([window accessibilityAttributeValue:@"AXFocusedUIElement"] == content);
        }
        id app = [GPUIApplication new];
        assert([app accessibilityFocusedUIElement] == app);
        id panel = [GPUIPanel new];
        [panel setContentView:content];
        [app setKeyWindow:panel];
        assert([app accessibilityFocusedUIElement] == content);
    }
}
'''
        with tempfile.TemporaryDirectory(prefix='murmur-focus-hook-') as directory:
            path = Path(directory)
            (path / 'test.m').write_text(source)
            subprocess.run(['clang', '-fobjc-arc', '-Werror', '-framework', 'AppKit',
                            str(ROOT / 'app/gpui/native/ax_focus_fix.m'),
                            str(path / 'test.m'), '-o', str(path / 'test')], check=True)
            subprocess.run([str(path / 'test')], check=True)

    def test_overlay_material_is_a_sibling_below_metal_and_never_takes_hits(self):
        source = r'''
#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>
#include <assert.h>
extern void *murmur_overlay_material(void *view);
extern void murmur_overlay_refit_hud(void *view, double width, double height, double radius);
int main(void) {
    @autoreleasepool {
        NSView *container = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 241, 98)];
        NSView *metal = [[NSView alloc] initWithFrame:container.bounds];
        metal.wantsLayer = YES;
        CALayer *layer = metal.layer;
        [container addSubview:metal];
        void *raw = (__bridge void *)metal;
        NSVisualEffectView *hud = (__bridge NSVisualEffectView *)murmur_overlay_material(raw);
        assert(hud && hud.superview == metal.superview);
        assert(container.subviews.count == 2 && container.subviews[0] == hud && container.subviews[1] == metal);
        assert(metal.subviews.count == 0 && metal.layer == layer);
        assert(murmur_overlay_material(raw) == (__bridge void *)hud);
        assert([hud hitTest:NSMakePoint(10, 10)] == nil);
        murmur_overlay_refit_hud(raw, 241, 98, 12);
        assert(NSEqualRects(hud.frame, metal.frame));
        // A shrink retains a centered bottom-anchored material, then exact bounds.
        murmur_overlay_refit_hud(raw, 120, 34, 17);
        assert(NSEqualRects(hud.frame, NSMakeRect(60.5, 0, 120, 34)));
        metal.frame = NSMakeRect(0, 0, 120, 34);
        murmur_overlay_refit_hud(raw, 120, 34, 17);
        assert(NSEqualRects(hud.frame, metal.frame));
        // Reparenting/placement changes must preserve ordering and conversion.
        NSView *other = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 500, 200)];
        [metal removeFromSuperview];
        metal.frame = NSMakeRect(25, 30, 241, 98);
        [other addSubview:metal];
        murmur_overlay_refit_hud(raw, 241, 98, 12);
        assert(hud.superview == other && other.subviews[0] == hud && other.subviews[1] == metal);
        assert(NSEqualRects(hud.frame, metal.frame));
    }
}
'''
        with tempfile.TemporaryDirectory(prefix='murmur-overlay-hierarchy-') as directory:
            path = Path(directory)
            (path / 'test.m').write_text(source)
            subprocess.run(['clang', '-fobjc-arc', '-Werror', '-framework', 'AppKit',
                            '-framework', 'QuartzCore',
                            str(ROOT / 'app/gpui/native/overlay.m'),
                            str(path / 'test.m'), '-o', str(path / 'test')], check=True)
            subprocess.run([str(path / 'test')], check=True)

    def test_overlay_never_key_hook_is_instance_local_and_idempotent(self):
        source = r'''
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <assert.h>
#include <stdbool.h>
@interface SyntheticPanel : NSObject
@end
@implementation SyntheticPanel
- (BOOL)canBecomeKeyWindow { return YES; }
- (BOOL)canBecomeMainWindow { return YES; }
@end
extern bool murmur_overlay_never_key(void *object);
int main(void) {
    @autoreleasepool {
        SyntheticPanel *overlay = [SyntheticPanel new];
        SyntheticPanel *other = [SyntheticPanel new];
        Class original = object_getClass(overlay);
        assert(murmur_overlay_never_key((__bridge void *)overlay));
        assert(murmur_overlay_never_key((__bridge void *)overlay));
        assert(!overlay.canBecomeKeyWindow && !overlay.canBecomeMainWindow);
        assert(other.canBecomeKeyWindow && other.canBecomeMainWindow);
        assert(object_getClass(other) == original);
        assert(!murmur_overlay_never_key(NULL));
    }
}
'''
        with tempfile.TemporaryDirectory(prefix='murmur-overlay-hook-') as directory:
            path = Path(directory)
            (path / 'test.m').write_text(source)
            subprocess.run(['clang', '-fobjc-arc', '-Werror', '-framework', 'AppKit',
                            '-framework', 'QuartzCore',
                            str(ROOT / 'app/gpui/native/overlay.m'),
                            str(path / 'test.m'), '-o', str(path / 'test')], check=True)
            subprocess.run([str(path / 'test')], check=True)


if __name__ == '__main__':
    unittest.main()
