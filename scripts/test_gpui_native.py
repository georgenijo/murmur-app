"""Compile the focus shim against inert classes; never create AppKit windows."""
from pathlib import Path
import platform
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(platform.system() == 'Darwin', 'Objective-C runtime requires macOS')
class FocusHookTests(unittest.TestCase):
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
@interface GPUIWindow : NSObject
@property id contentView;
@end
@implementation GPUIWindow
@end
@interface GPUIPanel : NSObject
@property id contentView;
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
            [window setContentView:content];
            assert([window accessibilityFocusedUIElement] == content);
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

    def test_panel_configuration_is_applied_once_across_frame_transactions(self):
        source = r'''
#import <AppKit/AppKit.h>
#include <assert.h>
#include <stdbool.h>
@interface SyntheticWindow : NSObject
@property(nonatomic) NSInteger level;
@property(nonatomic) BOOL hasShadow;
@property(nonatomic) BOOL opaque;
@property(nonatomic) NSUInteger opaqueWrites;
@property(nonatomic,strong) NSColor *backgroundColor;
@property(nonatomic) NSWindowCollectionBehavior collectionBehavior;
@property(nonatomic) BOOL ignoresMouseEvents;
@property(nonatomic) NSRect frame;
@property(nonatomic) double alphaValue;
@end
@implementation SyntheticWindow
- (BOOL)canBecomeKeyWindow { return YES; }
- (BOOL)canBecomeMainWindow { return YES; }
- (void)setOpaque:(BOOL)value { _opaque=value; self.opaqueWrites+=1; }
- (void)setFrame:(NSRect)value display:(BOOL)display { (void)display; self.frame=value; }
@end
@interface SyntheticView : NSObject
@property(nonatomic,strong) SyntheticWindow *window;
@end
@implementation SyntheticView
@end
extern bool murmur_overlay_apply(void *, double, double, double, double, bool, bool, bool, bool, double, double);
int main(void) {
    @autoreleasepool {
        SyntheticView *view = [SyntheticView new];
        view.window = [SyntheticWindow new];
        assert(murmur_overlay_apply((__bridge void *)view, 0,0,200,42,false,true,true,true,14,42));
        assert(murmur_overlay_apply((__bridge void *)view, 0,0,220,42,false,false,true,true,14,42));
        assert(view.window.opaqueWrites == 1);
        assert(view.window.frame.size.width == 220);
        assert(view.window.alphaValue == 1);
        assert(!view.window.canBecomeKeyWindow && !view.window.canBecomeMainWindow);
    }
}
'''
        with tempfile.TemporaryDirectory(prefix='murmur-panel-transaction-') as directory:
            path = Path(directory)
            (path / 'test.m').write_text(source)
            subprocess.run(['clang', '-fobjc-arc', '-Werror', '-framework', 'AppKit',
                            '-framework', 'QuartzCore',
                            str(ROOT / 'app/gpui/native/overlay.m'),
                            str(path / 'test.m'), '-o', str(path / 'test')], check=True)
            subprocess.run([str(path / 'test')], check=True)

    def test_preview_is_click_through_even_while_visible(self):
        source = r'''
#import <AppKit/AppKit.h>
#include <assert.h>
#include <stdbool.h>
@interface SyntheticWindow : NSObject
@property BOOL ignoresMouseEvents;
@property(getter=isVisible) BOOL visible;
@property NSUInteger frontCount;
@end
@implementation SyntheticWindow
- (void)orderOut:(id)sender { (void)sender; self.visible=NO; }
- (void)orderFrontRegardless { self.visible=YES; self.frontCount+=1; }
@end
@interface SyntheticView : NSObject
@property SyntheticWindow *window;
@end
@implementation SyntheticView
@end
extern void murmur_preview_refit(void *, double, double, double, double);
extern void murmur_preview_visibility(void *, bool);
int main(void) {
    @autoreleasepool {
        SyntheticView *view = [SyntheticView new];
        view.window = [SyntheticWindow new];
        view.window.ignoresMouseEvents = NO;
        murmur_preview_refit((__bridge void *)view, 12, 64, 200, 42);
        assert(view.window.ignoresMouseEvents);
        view.window.ignoresMouseEvents = NO;
        murmur_preview_refit((__bridge void *)view, 12, 0, 100, 42);
        assert(view.window.ignoresMouseEvents);
        murmur_preview_visibility((__bridge void *)view, false);
        murmur_preview_visibility((__bridge void *)view, false);
        assert(view.window.visible && view.window.frontCount == 1);
        assert(view.window.ignoresMouseEvents);
        murmur_preview_visibility((__bridge void *)view, true);
        assert(!view.window.visible && view.window.ignoresMouseEvents);
    }
}
'''
        with tempfile.TemporaryDirectory(prefix='murmur-preview-hook-') as directory:
            path = Path(directory)
            (path / 'test.m').write_text(source)
            subprocess.run(['clang', '-fobjc-arc', '-Werror', '-framework', 'AppKit',
                            '-framework', 'QuartzCore',
                            str(ROOT / 'app/gpui/native/overlay.m'),
                            str(path / 'test.m'), '-o', str(path / 'test')], check=True)
            subprocess.run([str(path / 'test')], check=True)


if __name__ == '__main__':
    unittest.main()
