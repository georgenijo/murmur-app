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

static char hud_key;
static char panel_setup_key;
bool murmur_overlay_apply(void *raw, double x, double y, double width, double height,
                          bool floating, bool hidden, bool reduced_transparency,
                          bool dark, double radius, double surface_height) {
    assert([NSThread isMainThread]);
    NSView *view = (__bridge NSView *)raw;
    NSWindow *window = view.window;
    if (!window || !murmur_overlay_never_key((__bridge void *)window)) return false;
    // AppKit setup is instance-local and idempotent. Reinstalling style/opacity
    // during a presentation callback can reenter the platform's frame update.
    if (!objc_getAssociatedObject(window, &panel_setup_key)) {
        window.level = 25;
        window.hasShadow = NO;
        window.opaque = NO;
        window.backgroundColor = NSColor.clearColor;
        window.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces |
            NSWindowCollectionBehaviorStationary | NSWindowCollectionBehaviorIgnoresCycle |
            NSWindowCollectionBehaviorFullScreenAuxiliary | NSWindowCollectionBehaviorCanJoinAllApplications;
        if ([window isKindOfClass:NSPanel.class]) {
            NSPanel *panel = (NSPanel *)window;
            if (!(panel.styleMask & NSWindowStyleMaskNonactivatingPanel)) {
                panel.styleMask |= NSWindowStyleMaskNonactivatingPanel;
            }
            panel.hidesOnDeactivate = NO;
            panel.becomesKeyOnlyIfNeeded = YES;
        }
        SEL prevent = NSSelectorFromString(@"_setPreventsActivation:");
        if ([window respondsToSelector:prevent]) {
            void (*setter)(id, SEL, BOOL) = (void *)[window methodForSelector:prevent];
            setter(window, prevent, YES);
        }
        objc_setAssociatedObject(window, &panel_setup_key, @YES, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    }
    window.ignoresMouseEvents = hidden;
    // Input uses global top-left coordinates; AppKit uses the primary screen's
    // bottom-left origin. Never derive this from the window's stale display.
    NSScreen *primary = NSScreen.screens.firstObject;
    double primary_top = NSMaxY(primary.frame);
    NSRect target = NSMakeRect(x, primary_top-y-height, width, height);
    if (!NSEqualRects(window.frame,target)) [window setFrame:target display:YES];
    window.alphaValue = hidden ? 0.0 : 1.0;
    NSVisualEffectView *hud = objc_getAssociatedObject(view, &hud_key);
    if (!floating) {
        [hud removeFromSuperview];
        objc_setAssociatedObject(view, &hud_key, nil, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
        return true;
    }
    if (!hud) {
        hud = [[NSVisualEffectView alloc] initWithFrame:view.bounds];
        hud.material = NSVisualEffectMaterialHUDWindow;
        hud.blendingMode = NSVisualEffectBlendingModeBehindWindow;
        hud.state = NSVisualEffectStateActive;
        hud.wantsLayer = YES;
        // This is the raw content view, not a WKWebView. No WebKit exists here.
        [view addSubview:hud positioned:NSWindowBelow relativeTo:nil];
        objc_setAssociatedObject(view, &hud_key, hud, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    }
    hud.appearance = [NSAppearance appearanceNamed:dark ? NSAppearanceNameDarkAqua : NSAppearanceNameAqua];
    hud.frame = NSMakeRect(0, view.isFlipped ? view.bounds.size.height-surface_height : 0, view.bounds.size.width, surface_height);
    hud.layer.cornerRadius = radius;
    hud.layer.masksToBounds = YES;
    hud.layer.borderWidth = 1.0;
    hud.layer.borderColor = [NSColor colorWithWhite:dark ? 1.0 : 0.0 alpha:0.10].CGColor;
    hud.layer.backgroundColor = [NSColor colorWithSRGBRed:(dark ? 28.0 : 246.0)/255.0
        green:(dark ? 28.0 : 246.0)/255.0 blue:(dark ? 30.0 : 248.0)/255.0
        alpha:reduced_transparency ? 1.0 : (dark ? 0.74 : 0.76)].CGColor;
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
    // Overlay callers send only content-free state labels. The explicit
    // preview-only wrapper preserves the existing consent-gated polite live
    // region's speech. GPUI's pinned AccessKit lacks a live-region API.
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

void murmur_overlay_refit_hud(void *raw, double width, double height, double radius) {
    assert([NSThread isMainThread]);
    NSView *view=(__bridge NSView *)raw;
    NSVisualEffectView *hud=objc_getAssociatedObject(view,&hud_key);
    if (!hud) return;
    hud.frame=NSMakeRect((view.bounds.size.width-width)/2.0,view.isFlipped ? view.bounds.size.height-height : 0,width,height);
    hud.layer.cornerRadius=radius;
}

// Preview reuses the overlay's never-key/main-thread panel transaction, but
// ignores every pointer event even while visible. Mask only the painted card.
void murmur_preview_refit(void *raw, double x, double y, double width, double height) {
    assert([NSThread isMainThread]);
    NSView *view=(__bridge NSView *)raw;
    view.window.ignoresMouseEvents=YES;
    NSVisualEffectView *hud=objc_getAssociatedObject(view,&hud_key);
    if (!hud) return;
    hud.frame=NSMakeRect(x, view.isFlipped ? y : view.bounds.size.height-y-height, width, height);
    hud.layer.cornerRadius=14.;
}

// A preview starts ordered out. Admit only after its nonempty frame and the
// exact recording/consent ticket have been checked in the main-thread owner.
void murmur_preview_visibility(void *raw, bool hidden) {
    assert([NSThread isMainThread]);
    NSView *view=(__bridge NSView *)raw;
    NSWindow *window=view.window;
    window.ignoresMouseEvents=YES;
    if (hidden) [window orderOut:nil];
    else if (!window.isVisible) [window orderFrontRegardless];
}
