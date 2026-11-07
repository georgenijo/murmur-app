"""Compile native shims against synthetic views/panels; never show windows."""
from pathlib import Path
import platform
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(platform.system() == 'Darwin', 'Objective-C runtime requires macOS')
class FocusHookTests(unittest.TestCase):
    def test_preview_normalizes_a_default_resizable_panel_before_presentation(self):
        source = r'''
#import <AppKit/AppKit.h>
#include <assert.h>
@interface SyntheticPreviewPanel : NSPanel
@property NSUInteger styleWrites;
@end
@implementation SyntheticPreviewPanel
- (void)setStyleMask:(NSWindowStyleMask)mask { self.styleWrites++; [super setStyleMask:mask]; }
@end
extern void murmur_preview_refit(void *, double, double, double, double);
int main(void) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        NSWindowStyleMask mask = NSWindowStyleMaskTitled | NSWindowStyleMaskResizable |
            NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskNonactivatingPanel;
        SyntheticPreviewPanel *panel = [[SyntheticPreviewPanel alloc] initWithContentRect:NSMakeRect(0,0,460,168)
            styleMask:mask backing:NSBackingStoreBuffered defer:NO];
        NSPanel *other = [[NSPanel alloc] initWithContentRect:NSMakeRect(0,0,460,168)
            styleMask:mask backing:NSBackingStoreBuffered defer:NO];
        panel.styleWrites = 0;
        NSView *view = panel.contentView;
        murmur_preview_refit((__bridge void *)view, 120,0,220,42);
        assert(!(panel.styleMask & NSWindowStyleMaskResizable));
        assert(!(panel.styleMask & (NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable)));
        assert(panel.styleMask & NSWindowStyleMaskFullSizeContentView);
        assert(panel.styleMask & NSWindowStyleMaskNonactivatingPanel);
        assert(panel.titlebarAppearsTransparent && panel.titleVisibility == NSWindowTitleHidden);
        assert(!panel.movable && panel.ignoresMouseEvents && !panel.visible);
        assert(panel.contentView == view);
        assert(NSEqualSizes(view.frame.size, panel.frame.size));
        murmur_preview_refit((__bridge void *)view, 120,64,220,42);
        assert(panel.styleWrites == 1);
        assert(other.styleMask == mask && !other.ignoresMouseEvents && !other.visible);
    }
}
'''
        with tempfile.TemporaryDirectory(prefix='murmur-preview-panel-') as directory:
            path = Path(directory)
            (path / 'test.m').write_text(source)
            subprocess.run(['clang', '-fobjc-arc', '-Werror', '-framework', 'AppKit',
                            '-framework', 'QuartzCore', str(ROOT / 'app/gpui/native/overlay.m'),
                            str(path / 'test.m'), '-o', str(path / 'test')], check=True)
            subprocess.run([str(path / 'test')], check=True)

    def test_overlay_contraction_and_reentry_sample_the_painted_band_in_both_placements(self):
        source = r'''
#import <AppKit/AppKit.h>
#include <assert.h>
#include <stdbool.h>
@interface FlippedView : NSView
@end
@implementation FlippedView
- (BOOL)isFlipped { return YES; }
@end
extern void murmur_overlay_refit_hud(void *, double, double, double, bool);
extern bool murmur_overlay_surface_contains(void *, NSPoint);
static NSPoint from_top(NSView *view, double x, double y) {
    return NSMakePoint(x, view.isFlipped ? y : view.bounds.size.height-y);
}
int main(void) {
    @autoreleasepool {
        for (Class cls in @[NSView.class, FlippedView.class]) {
            for (int floating = 0; floating <= 1; floating++) {
                // No HUD is installed for a notch. Presence still tracks its
                // painted rectangle, not the retained expanded window bounds.
                NSView *view = [[cls alloc] initWithFrame:NSMakeRect(0, 0, 241, 101)];
                void *raw = (__bridge void *)view;
                NSPoint top = from_top(view, 120.5, 10);
                NSPoint bottom = from_top(view, 120.5, 90);
                NSPoint persistent = floating ? bottom : top;
                NSPoint disappearing = floating ? top : bottom;
                double collapsed = floating ? 34 : 37;
                assert(!murmur_overlay_surface_contains(raw, persistent));
                murmur_overlay_refit_hud(raw, 241, 101, 12, floating);
                assert(murmur_overlay_surface_contains(raw, persistent));
                assert(murmur_overlay_surface_contains(raw, disappearing));
                // About 100 ms into contraction: 61 pt painted in 101 pt.
                murmur_overlay_refit_hud(raw, 200, 61, 12, floating);
                assert(!murmur_overlay_surface_contains(raw, disappearing));
                assert(murmur_overlay_surface_contains(raw, persistent));
                // Leaving through the transparent band, then re-entering the
                // persistent band, must produce outside -> inside in either
                // coordinate system. Horizontal transparent margins stay out.
                assert(!murmur_overlay_surface_contains(raw, from_top(view, 5, 50)));
                murmur_overlay_refit_hud(raw, 120, collapsed, 17, floating);
                assert(!murmur_overlay_surface_contains(raw, disappearing));
                assert(murmur_overlay_surface_contains(raw, persistent));
                // Commit the native shrink, then grow again. Both anchors
                // keep the persistent band at the window edge throughout.
                view.frame = NSMakeRect(0, 0, 120, collapsed);
                murmur_overlay_refit_hud(raw, 120, collapsed, 17, floating);
                assert(murmur_overlay_surface_contains(raw, from_top(view, 60, collapsed/2)));
                view.frame = NSMakeRect(0, 0, 241, 101);
                murmur_overlay_refit_hud(raw, 241, 101, 12, floating);
                assert(murmur_overlay_surface_contains(raw, persistent));
                assert(murmur_overlay_surface_contains(raw, disappearing));
            }
        }
    }
}
'''
        with tempfile.TemporaryDirectory(prefix='murmur-overlay-presence-') as directory:
            path = Path(directory)
            (path / 'test.m').write_text(source)
            subprocess.run(['clang', '-fobjc-arc', '-Werror', '-framework', 'AppKit',
                            '-framework', 'QuartzCore',
                            str(ROOT / 'app/gpui/native/overlay.m'),
                            str(path / 'test.m'), '-o', str(path / 'test')], check=True)
            subprocess.run([str(path / 'test')], check=True)

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
#include <stdbool.h>
extern void *murmur_overlay_material(void *view);
extern void murmur_overlay_refit_hud(void *view, double width, double height, double radius, bool floating);
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
        murmur_overlay_refit_hud(raw, 241, 98, 12, true);
        assert(NSEqualRects(hud.frame, metal.frame));
        // A shrink retains a centered bottom-anchored material, then exact bounds.
        murmur_overlay_refit_hud(raw, 120, 34, 17, true);
        assert(NSEqualRects(hud.frame, NSMakeRect(60.5, 0, 120, 34)));
        metal.frame = NSMakeRect(0, 0, 120, 34);
        murmur_overlay_refit_hud(raw, 120, 34, 17, true);
        assert(NSEqualRects(hud.frame, metal.frame));
        // Reparenting/placement changes must preserve ordering and conversion.
        NSView *other = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 500, 200)];
        [metal removeFromSuperview];
        metal.frame = NSMakeRect(25, 30, 241, 98);
        [other addSubview:metal];
        murmur_overlay_refit_hud(raw, 241, 98, 12, true);
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

    def test_preview_surface_converts_card_coordinates_and_tracks_reparenting(self):
        source = r'''
#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>
#include <assert.h>
#include <stdbool.h>
@interface FlippedPreview : NSView
@end
@implementation FlippedPreview
- (BOOL)isFlipped { return YES; }
@end
extern void *murmur_overlay_material(void *);
extern void murmur_preview_refit(void *, double, double, double, double);
extern bool murmur_overlay_surface_contains(void *, NSPoint);
int main(void) {
    @autoreleasepool {
        for (Class parentClass in @[NSView.class, FlippedPreview.class]) {
            for (Class viewClass in @[NSView.class, FlippedPreview.class]) {
                NSView *parent = [[parentClass alloc] initWithFrame:NSMakeRect(0, 0, 500, 400)];
                NSView *view = [[viewClass alloc] initWithFrame:NSMakeRect(25, 30, 300, 160)];
                view.bounds = NSMakeRect(7, 9, 300, 160);
                [parent addSubview:view];
                void *raw = (__bridge void *)view;
                NSVisualEffectView *hud = (__bridge NSVisualEffectView *)murmur_overlay_material(raw);
                for (int lifted = 0; lifted <= 1; lifted++) {
                    double y = lifted ? 12 : 64;
                    murmur_preview_refit(raw, 18, y, 210, 42);
                    NSRect card = NSMakeRect(25, 9+(view.isFlipped ? y : 160-y-42), 210, 42);
                    assert(NSEqualRects(hud.frame, [view convertRect:card toView:parent]));
                    assert(hud.layer.cornerRadius == 14);
                    assert(murmur_overlay_surface_contains(raw, NSMakePoint(NSMidX(card), NSMidY(card))));
                    assert(!murmur_overlay_surface_contains(raw, NSMakePoint(8, 10)));
                    assert([hud hitTest:NSMakePoint(30, 30)] == nil);
                }
                NSView *other = [[parentClass alloc] initWithFrame:NSMakeRect(0, 0, 600, 500)];
                [view removeFromSuperview];
                [other addSubview:view];
                murmur_preview_refit(raw, 18, 64, 210, 42);
                assert(hud.superview == other && other.subviews[0] == hud && other.subviews[1] == view);
                assert(NSEqualRects(hud.frame, [view convertRect:NSMakeRect(25, 9+(view.isFlipped ? 64 : 54), 210, 42) toView:other]));
                // Notch previews have no material, but retain their own surface.
                NSView *notch = [[viewClass alloc] initWithFrame:NSMakeRect(0, 0, 300, 160)];
                murmur_preview_refit((__bridge void *)notch, 18, 64, 210, 42);
                assert(murmur_overlay_surface_contains((__bridge void *)notch, NSMakePoint(100, notch.isFlipped ? 80 : 70)));
            }
        }
    }
}
'''
        with tempfile.TemporaryDirectory(prefix='murmur-preview-surface-') as directory:
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
@property(nonatomic,strong) SyntheticWindow *syntheticWindow;
@end
@implementation SyntheticView
- (NSWindow *)window { return (NSWindow *)self.syntheticWindow; }
- (NSRect)bounds { return NSMakeRect(0, 0, 240, 106); }
- (BOOL)isFlipped { return NO; }
@end
extern bool murmur_overlay_apply(void *, double, double, double, double, bool, bool, bool, bool, bool, double, double);
extern void murmur_preview_refit(void *, double, double, double, double);
int main(void) {
    @autoreleasepool {
        SyntheticView *view = [SyntheticView new];
        view.syntheticWindow = [SyntheticWindow new];
        assert(murmur_overlay_apply((__bridge void *)view, 0,0,200,42,false,true,true,false,true,14,42));
        assert(murmur_overlay_apply((__bridge void *)view, 0,0,220,42,false,false,true,false,true,14,42));
        assert(view.syntheticWindow.opaqueWrites == 1);
        assert(view.syntheticWindow.frame.size.width == 220);
        assert(view.syntheticWindow.alphaValue == 1);
        assert(!view.syntheticWindow.ignoresMouseEvents);
        // The preview marker must survive the shared resize transaction.
        murmur_preview_refit((__bridge void *)view, 12, 0, 100, 42);
        assert(murmur_overlay_apply((__bridge void *)view, 0,0,240,42,false,false,true,false,true,14,42));
        assert(view.syntheticWindow.ignoresMouseEvents);
        assert(view.syntheticWindow.opaqueWrites == 1);
        assert(!view.syntheticWindow.canBecomeKeyWindow && !view.syntheticWindow.canBecomeMainWindow);
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
@property SyntheticWindow *syntheticWindow;
@end
@implementation SyntheticView
- (NSWindow *)window { return (NSWindow *)self.syntheticWindow; }
- (NSRect)bounds { return NSMakeRect(0, 0, 240, 106); }
- (BOOL)isFlipped { return NO; }
@end
extern void murmur_preview_refit(void *, double, double, double, double);
extern void murmur_preview_visibility(void *, bool);
int main(void) {
    @autoreleasepool {
        SyntheticView *view = [SyntheticView new];
        view.syntheticWindow = [SyntheticWindow new];
        view.syntheticWindow.ignoresMouseEvents = NO;
        murmur_preview_refit((__bridge void *)view, 12, 64, 200, 42);
        assert(view.syntheticWindow.ignoresMouseEvents);
        view.syntheticWindow.ignoresMouseEvents = NO;
        murmur_preview_refit((__bridge void *)view, 12, 0, 100, 42);
        assert(view.syntheticWindow.ignoresMouseEvents);
        murmur_preview_visibility((__bridge void *)view, false);
        murmur_preview_visibility((__bridge void *)view, false);
        assert(view.syntheticWindow.visible && view.syntheticWindow.frontCount == 1);
        assert(view.syntheticWindow.ignoresMouseEvents);
        murmur_preview_visibility((__bridge void *)view, true);
        assert(!view.syntheticWindow.visible && view.syntheticWindow.ignoresMouseEvents);
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


class WebkitSourcePolicyTests(unittest.TestCase):
    def test_attributed_inventory_parser(self):
        from scripts.check_gpui_goldens import check_inventory
        import json
        valid = dict(pid=1, webkitImages=5, axAttributed=5, unattributed=0,
                     startupWebkitImages=0, linkageViolations=0)
        def parse(value):
            return check_inventory('MURMUR_TEST_INVENTORY=' + json.dumps(value),
                                   'MURMUR_TEST_INVENTORY')
        self.assertEqual(parse(valid), valid)
        for field, value in [('unattributed', 1), ('axAttributed', 4),
                             ('startupWebkitImages', 1), ('linkageViolations', 1),
                             ('webkitImages', -1), ('pid', True)]:
            with self.subTest(field=field), self.assertRaises(AssertionError):
                parse({**valid, field: value})
        with self.assertRaises(AssertionError):
            check_inventory('', 'MURMUR_TEST_INVENTORY')

    def test_gpui_sources_do_not_construct_web_views(self):
        # No linked class reference, dynamic named lookup, or message dispatch
        # to a web-view class is allowed in our GPUI sources. The policy's own
        # classifier strings are evidence matching, not class construction.
        root = ROOT / 'app/gpui'
        for directory in ('src', 'native'):
            for path in (root / directory).rglob('*'):
                if path.suffix not in ('.rs', '.m', '.h') or path.name == 'webkit_policy.m':
                    continue
                self.assertNotRegex(path.read_text(), r'WKWebView|\bWebView\b', str(path))


@unittest.skipUnless(platform.system() == 'Darwin', 'dyld policy requires macOS')
class WebkitPolicyTests(unittest.TestCase):
    def test_classifier_linkage_and_zero_baseline(self):
        source = r'''
#include "webkit_policy.m"
#include <assert.h>
int main(void) {
    assert(murmur_webkit_start());
    assert(murmur_webkit_inventory("MURMUR_TEST_INVENTORY"));
    // Synthetic chain follows the captured VoiceOver/Apple AX bundle stack.
    WebkitFrame ax[] = {
        {"/usr/lib/dyld", "dlopen"},
        {"/System/Library/PrivateFrameworks/SoftLinking.framework/SoftLinking", "_sl_dlopen"},
        {"/System/Library/PrivateFrameworks/AccessibilityBundles.framework/AccessibilityBundles",
            "-[AXBBundleManager _loadCoreAccessibilityAdditionsForBundle:]"},
        {"/System/Library/Frameworks/ApplicationServices.framework/Frameworks/HIServices.framework/HIServices",
            "loadAccessibilityBundles"},
        {"/System/Library/Frameworks/ApplicationServices.framework/Frameworks/HIServices.framework/HIServices",
            "_AXXMIGSetAttributeValue"},
        {"/System/Library/Frameworks/ApplicationServices.framework/Frameworks/HIServices.framework/HIServices",
            "mshMIGPerform"},
        {"/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation",
            "__CFRUNLOOP_IS_CALLING_OUT_TO_A_SOURCE1_PERFORM_FUNCTION__"},
        {"/System/Library/Frameworks/AppKit.framework/AppKit", "-[NSApplication run]"},
        {"/synthetic/Murmur", "main"},
    };
    unsigned n=sizeof(ax)/sizeof(*ax);
    assert(murmur_webkit_ax_chain(ax,n,false));
    assert(!murmur_webkit_ax_chain(ax,n,true)); // Truncated evidence.
    assert(!murmur_webkit_ax_chain(ax,0,false));
    assert(!murmur_webkit_ax_chain(ax,4,false)); // No incoming AX IPC.
    assert(!murmur_webkit_ax_chain(ax,5,false)); // No Mach/runloop dispatch.
    WebkitFrame dispatch=ax[5];
    strcpy(ax[5].image,"/synthetic/Murmur");
    strcpy(ax[5].symbol,"direct_AX_call");
    assert(!murmur_webkit_ax_chain(ax,n,false)); // App invoking the AX handler itself.
    ax[5]=dispatch;
    WebkitFrame saved=ax[1];
    strcpy(ax[1].image,"/synthetic/Murmur");
    strcpy(ax[1].symbol,"create_web_view");
    assert(!murmur_webkit_ax_chain(ax,n,false)); // App initiated, even with AX frames.
    ax[1]=saved;
    ax[1].image[0]=0;
    assert(!murmur_webkit_ax_chain(ax,n,false)); // Unknown origin.
    ax[1]=saved;
    strcpy(ax[1].image,"/tmp/AccessibilityBundles.framework/AccessibilityBundles");
    assert(!murmur_webkit_ax_chain(ax,n,false)); // Untrusted path spoof.
    ax[1]=saved;
    strcpy(ax[n-1].symbol,"-[WKWebView initWithFrame:]");
    assert(!murmur_webkit_ax_chain(ax,n,false)); // Web view anywhere in chain.
    assert(murmur_webkit_family("/System/Library/Frameworks/WebKit.framework/Versions/A/WebKit"));
    assert(murmur_webkit_family("@rpath/JavaScriptCore.framework/JavaScriptCore"));
    assert(!murmur_webkit_family("/System/Library/Frameworks/AppKit.framework/AppKit"));
    struct { struct mach_header_64 h; struct dylib_command d; char name[128]; } image={0};
    image.h.magic=MH_MAGIC_64; image.h.ncmds=1;
    image.h.sizeofcmds=sizeof(image)-sizeof(image.h);
    image.d.cmd=LC_LOAD_DYLIB; image.d.cmdsize=image.h.sizeofcmds;
    image.d.dylib.name.offset=sizeof(image.d);
    strcpy(image.name,"/System/Library/Frameworks/AppKit.framework/AppKit");
    assert(!links_webkit((void *)&image));
    strcpy(image.name,"@rpath/WebKit.framework/WebKit");
    assert(links_webkit((void *)&image));
    image.d.cmd=LC_LOAD_WEAK_DYLIB;
    assert(links_webkit((void *)&image));
    image.d.cmd=LC_REEXPORT_DYLIB;
    assert(links_webkit((void *)&image));
    image.d.dylib.name.offset=image.d.cmdsize;
    assert(links_webkit((void *)&image)); // Malformed linkage fails closed.
    // A real app-owned load must fail despite a clean startup. No web view,
    // NSApplication, persistent store or accessibility client is constructed.
    assert(dlopen("/System/Library/Frameworks/WebKit.framework/WebKit",RTLD_LAZY|RTLD_LOCAL));
    assert(!murmur_webkit_inventory("MURMUR_TEST_APP_LOAD_INVENTORY"));
}
'''
        with tempfile.TemporaryDirectory(prefix='murmur-webkit-policy-') as directory:
            path = Path(directory)
            (path / 'test.m').write_text(source)
            subprocess.run(['clang', '-fobjc-arc', '-fno-omit-frame-pointer', '-Werror',
                            '-framework', 'Foundation', '-I', str(ROOT / 'app/gpui/native'),
                            str(path / 'test.m'), '-o', str(path / 'test')], check=True)
            subprocess.run([str(path / 'test')], check=True)


if __name__ == '__main__':
    unittest.main()
