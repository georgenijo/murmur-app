// Phase 0a treatment: instance-local never-key panel, prevents activation,
// level 25/all Spaces, and a behind-window HUD refit in the frame transaction.
#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>
#import <objc/runtime.h>
#include <stdbool.h>
#include <stdint.h>
#include <assert.h>

static BOOL never_key(id self, SEL cmd) { (void)self; (void)cmd; return NO; }
// Exported separately so inert runtime tests exercise the production hook.
bool murmur_overlay_never_key(void *object) {
    assert([NSThread isMainThread]);
    id panel = (__bridge id)object;
    if (!panel) return false;
    Class original = object_getClass(panel);
    NSString *name = [@"MurmurNonactivating_" stringByAppendingString:NSStringFromClass(original)];
    if ([NSStringFromClass(original) hasPrefix:@"MurmurNonactivating_"]) return true;
    Class subclass = NSClassFromString(name);
    if (!subclass) {
        subclass = objc_allocateClassPair(original, name.UTF8String, 0);
        if (!subclass) return false;
        const char *keyType = method_getTypeEncoding(class_getInstanceMethod(original, @selector(canBecomeKeyWindow)));
        const char *mainType = method_getTypeEncoding(class_getInstanceMethod(original, @selector(canBecomeMainWindow)));
        class_addMethod(subclass, @selector(canBecomeKeyWindow), (IMP)never_key, keyType ?: "B@:");
        class_addMethod(subclass, @selector(canBecomeMainWindow), (IMP)never_key, mainType ?: "B@:");
        objc_registerClassPair(subclass);
    }
    object_setClass(panel, subclass);
    return true;
}

// GPUI draws into the parent view's Metal layer. Any child of that view,
// even NSWindowBelow, composites ABOVE the Metal foreground. Keep material
// and renderer as siblings; the material must never intercept input.
@interface MurmurOverlayMaterial : NSVisualEffectView
@end
@implementation MurmurOverlayMaterial
- (NSView *)hitTest:(NSPoint)point { (void)point; return nil; }
@end
static char hud_key, surface_key;

// Shared by the native transaction and the windowless hierarchy regression.
void *murmur_overlay_material(void *raw) {
    assert([NSThread isMainThread]);
    NSView *view = (__bridge NSView *)raw;
    MurmurOverlayMaterial *hud = objc_getAssociatedObject(view, &hud_key);
    if (!hud && view.superview) {
        hud = [[MurmurOverlayMaterial alloc] initWithFrame:NSZeroRect];
        hud.material = NSVisualEffectMaterialHUDWindow;
        hud.blendingMode = NSVisualEffectBlendingModeBehindWindow;
        hud.state = NSVisualEffectStateActive;
        hud.wantsLayer = YES;
        [view.superview addSubview:hud positioned:NSWindowBelow relativeTo:view];
        objc_setAssociatedObject(view, &hud_key, hud, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    }
    return (__bridge void *)hud;
}

static void refit_surface(NSView *view, double width, double height, double radius, bool floating) {
    // GPUI bottom-aligns floating pills and top-aligns notch cards. During a
    // contraction the window still has its expanded height: sample only the
    // painted band, in the view's own (possibly flipped) coordinates.
    NSRect surface = NSMakeRect(NSMinX(view.bounds)+(view.bounds.size.width-width)/2.0,
        NSMinY(view.bounds)+((view.isFlipped == floating) ? view.bounds.size.height-height : 0),
        width, height);
    objc_setAssociatedObject(view, &surface_key, [NSValue valueWithRect:surface], OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    MurmurOverlayMaterial *hud = objc_getAssociatedObject(view, &hud_key);
    if (!hud) return;
    if (hud.superview != view.superview) {
        [hud removeFromSuperview];
        [view.superview addSubview:hud positioned:NSWindowBelow relativeTo:view];
    }
    hud.frame = [view convertRect:surface toView:view.superview];
    hud.layer.cornerRadius = radius;
}
bool murmur_overlay_apply(void *raw, double x, double y, double width, double height,
                          bool floating, bool hidden, bool reduced_transparency,
                          bool high_contrast, bool dark, double radius, double surface_height) {
    assert([NSThread isMainThread]);
    NSView *view = (__bridge NSView *)raw;
    NSWindow *window = view.window;
    if (!window || !murmur_overlay_never_key((__bridge void *)window)) return false;
    window.level = 25;
    window.hasShadow = NO;
    window.opaque = NO;
    window.backgroundColor = NSColor.clearColor;
    window.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces |
        NSWindowCollectionBehaviorStationary | NSWindowCollectionBehaviorIgnoresCycle |
        NSWindowCollectionBehaviorFullScreenAuxiliary | NSWindowCollectionBehaviorCanJoinAllApplications;
    if ([window isKindOfClass:NSPanel.class]) {
        NSPanel *panel = (NSPanel *)window;
        panel.styleMask |= NSWindowStyleMaskNonactivatingPanel;
        panel.hidesOnDeactivate = NO;
        panel.becomesKeyOnlyIfNeeded = YES;
    }
    SEL prevent = NSSelectorFromString(@"_setPreventsActivation:");
    if ([window respondsToSelector:prevent]) {
        void (*setter)(id, SEL, BOOL) = (void *)[window methodForSelector:prevent];
        setter(window, prevent, YES);
    }
    window.ignoresMouseEvents = hidden;
    // Input uses global top-left coordinates; AppKit uses the primary screen's
    // bottom-left origin. Never derive this from the window's stale display.
    NSScreen *primary = NSScreen.screens.firstObject;
    double primary_top = NSMaxY(primary.frame);
    NSRect target = NSMakeRect(x, primary_top-y-height, width, height);
    if (!NSEqualRects(window.frame,target)) [window setFrame:target display:YES];
    window.alphaValue = hidden ? 0.0 : 1.0;
    MurmurOverlayMaterial *hud = objc_getAssociatedObject(view, &hud_key);
    if (!floating) {
        [hud removeFromSuperview];
        objc_setAssociatedObject(view, &hud_key, nil, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
        refit_surface(view, view.bounds.size.width, surface_height, radius, floating);
        return true;
    }
    hud = (__bridge MurmurOverlayMaterial *)murmur_overlay_material(raw);
    NSString *appearance = high_contrast
        ? (dark ? NSAppearanceNameAccessibilityHighContrastDarkAqua : NSAppearanceNameAccessibilityHighContrastAqua)
        : (dark ? NSAppearanceNameDarkAqua : NSAppearanceNameAqua);
    hud.appearance = [NSAppearance appearanceNamed:appearance];
    // GPUI owns the tint/foreground above the effect. Opaque accessibility
    // surfaces do not need a backdrop effect at all.
    hud.hidden = reduced_transparency || high_contrast;
    refit_surface(view, view.bounds.size.width, surface_height, radius, floating);
    hud.layer.masksToBounds = YES;
    return true;
}

// No app names, window titles, AX clients, screen capture or permission requests.
// Read NSScreen geometry from the same main display as the current host.
bool murmur_overlay_screen(double *out) {
    assert([NSThread isMainThread]);
    NSScreen *screen = nil;
    for (NSScreen *candidate in NSScreen.screens) {
        if ([candidate.deviceDescription[@"NSScreenNumber"] unsignedIntValue] == CGMainDisplayID()) {screen=candidate;break;}
    }
    if (!screen) return false;
    double primary_top = NSMaxY(NSScreen.screens.firstObject.frame);
    NSRect f=screen.frame, w=screen.visibleFrame;
    out[0]=NSMinX(f);out[1]=primary_top-NSMaxY(f);out[2]=NSMaxX(f);out[3]=primary_top-NSMinY(f);
    out[4]=NSMinX(w);out[5]=primary_top-NSMaxY(w);out[6]=NSMaxX(w);out[7]=primary_top-NSMinY(w);
    out[8]=screen.safeAreaInsets.top;
    out[9]=screen.auxiliaryTopLeftArea.size.width;out[10]=screen.auxiliaryTopRightArea.size.width;
    out[11]=screen.backingScaleFactor;
    NSUserDefaults *dock=[[NSUserDefaults alloc] initWithSuiteName:@"com.apple.dock"];
    NSString *orientation=[dock stringForKey:@"orientation"];
    out[12]=[dock boolForKey:@"autohide"] && (!orientation || [orientation isEqualToString:@"bottom"]) ? 1.0 : 0.0;
    out[13]=[dock objectForKey:@"tilesize"] ? [dock doubleForKey:@"tilesize"] : 64.0;
    out[14]=[dock boolForKey:@"magnification"] ? ([dock objectForKey:@"largesize"] ? [dock doubleForKey:@"largesize"] : 128.0) : 0.0;
    return true;
}
void murmur_overlay_announce(void *raw, const char *message) {
    assert([NSThread isMainThread]);
    NSView *view = (__bridge NSView *)raw;
    if (!view.window || !message) return;
    // Only a fixed content-free state label crosses this API, never words or a
    // calendar title. GPUI's pinned AccessKit lacks a live-region API.
    NSAccessibilityPostNotificationWithUserInfo(view, NSAccessibilityAnnouncementRequestedNotification,
        @{NSAccessibilityAnnouncementKey: [NSString stringWithUTF8String:message],
          NSAccessibilityPriorityKey: @(NSAccessibilityPriorityMedium)});
}

// Own-process inventory, usable by the offscreen host without NSApplication,
// windows, an AX client or inspecting any other process's data.
#include <mach-o/dyld.h>
#include <string.h>
uint32_t murmur_overlay_webkit_images(void) {
    uint32_t count=0;
    for (uint32_t i=0;i<_dyld_image_count();i++) {
        const char *name=_dyld_get_image_name(i);
        if (name && strstr(name,"/WebKit.framework/")) count++;
    }
    return count;
}

void murmur_overlay_refit_hud(void *raw, double width, double height, double radius, bool floating) {
    assert([NSThread isMainThread]);
    NSView *view=(__bridge NSView *)raw;
    refit_surface(view, width, height, radius, floating);
}

// Windowless regression tests use the same stored presentation rect as the
// real screen-pointer query, without moving the pointer or opening a window.
bool murmur_overlay_surface_contains(void *raw, NSPoint point) {
    assert([NSThread isMainThread]);
    NSView *view = (__bridge NSView *)raw;
    NSValue *surface = objc_getAssociatedObject(view, &surface_key);
    return surface && NSPointInRect(point, surface.rectValue);
}

// A never-key panel can miss tracking/GPUI exit events, especially across a
// resize. Read pointer presence without monitoring events or requiring AX.
bool murmur_overlay_pointer_inside(void *raw) {
    assert([NSThread isMainThread]);
    NSView *view = (__bridge NSView *)raw;
    if (!view.window || view.window.ignoresMouseEvents) return false;
    NSPoint point = [view.window convertPointFromScreen:NSEvent.mouseLocation];
    point = [view convertPoint:point fromView:nil];
    return murmur_overlay_surface_contains(raw, point);
}
