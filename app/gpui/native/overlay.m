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
static char hud_key, surface_key, panel_setup_key, preview_key, preview_setup_key;

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

static void refit_surface_rect(NSView *view, NSRect surface, double radius) {
    NSValue *previous = objc_getAssociatedObject(view, &surface_key);
    if (!previous || !NSEqualRects(previous.rectValue, surface)) {
        objc_setAssociatedObject(view, &surface_key, [NSValue valueWithRect:surface], OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    }
    MurmurOverlayMaterial *hud = objc_getAssociatedObject(view, &hud_key);
    if (!hud) return;
    if (hud.superview != view.superview) {
        [hud removeFromSuperview];
        [view.superview addSubview:hud positioned:NSWindowBelow relativeTo:view];
    }
    NSRect target = [view convertRect:surface toView:view.superview];
    if (!NSEqualRects(hud.frame, target)) hud.frame = target;
    if (hud.layer.cornerRadius != radius) hud.layer.cornerRadius = radius;
}

static void refit_surface(NSView *view, double width, double height, double radius, bool floating) {
    // GPUI bottom-aligns floating pills and top-aligns notch cards.
    NSRect surface = NSMakeRect(NSMinX(view.bounds)+(view.bounds.size.width-width)/2.0,
        NSMinY(view.bounds)+((view.isFlipped == floating) ? view.bounds.size.height-height : 0),
        width, height);
    refit_surface_rect(view, surface, radius);
}

bool murmur_overlay_apply(void *raw, double x, double y, double width, double height,
                          bool floating, bool hidden, bool reduced_transparency,
                          bool high_contrast, bool dark, double radius, double surface_height) {
    assert([NSThread isMainThread]);
    NSView *view = (__bridge NSView *)raw;
    NSWindow *window = view.window;
    if (!window) return false;
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
            // GPUI's titlebar:None popup is still Titled|FullSizeContentView.
            // AppKit then rounds/clips the TOP corners of the notch and can
            // constrain a first frame below the menu bar. Normalize before the
            // frame transaction/orderFront; only the painted surface has corners.
            panel.styleMask = (panel.styleMask | NSWindowStyleMaskNonactivatingPanel |
                NSWindowStyleMaskFullSizeContentView) & ~(NSWindowStyleMaskTitled |
                NSWindowStyleMaskResizable | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable);
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
    // Removing AppKit's titlebar tears down its KVO observers. Do this before
    // installing our per-instance never-key subclass, preserving KVO's isa
    // and observer bookkeeping throughout the style transaction.
    if (!murmur_overlay_never_key((__bridge void *)window)) return false;
    window.ignoresMouseEvents = hidden || objc_getAssociatedObject(view, &preview_key) != nil;
    // Input uses global top-left coordinates; AppKit uses the primary screen's
    // bottom-left origin. Never derive this from the window's stale display.
    NSScreen *primary = NSScreen.screens.firstObject;
    double primary_top = NSMaxY(primary.frame);
    NSRect target = NSMakeRect(x, primary_top-y-height, width, height);
    // Defer paint to GPUI's display-paced renderer. display:YES synchronously
    // paints from inside a borrowed GPUI frame and stalls the main thread.
    if (!NSEqualRects(window.frame,target)) [window setFrame:target display:NO];
    double alpha = hidden ? 0.0 : 1.0;
    if (window.alphaValue != alpha) window.alphaValue = alpha;
    // GPUI creates this companion with show:false. Present only after the
    // nonactivating policy and frame are installed, without keying the panel
    // or activating the app. Keep hidden surfaces persistent and click-through.
    if (!hidden && !window.isVisible) [window orderFrontRegardless];
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
    hud.hidden = hidden || reduced_transparency || high_contrast;
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

// Preview reuses the overlay's never-key/main-thread panel transaction, but
// ignores every pointer event even while visible. Mask only the painted card.
void murmur_preview_refit(void *raw, double x, double y, double width, double height) {
    assert([NSThread isMainThread]);
    NSView *view=(__bridge NSView *)raw;
    NSWindow *window=view.window;
    // A host may supply the default titled/resizable GPUI popup. On macOS,
    // its resize tracking region can exceed CGRegion's integer range;
    // CGRegionCreateWithRect returns NULL and AppKit unconditionally releases
    // it. Enforce the preview contract before its first resize/orderFront,
    // independently of whether the host used Preview::window_options.
    if ([window isKindOfClass:NSWindow.class] &&
        !objc_getAssociatedObject(window, &preview_setup_key)) {
        window.styleMask = (window.styleMask | NSWindowStyleMaskFullSizeContentView) &
            ~(NSWindowStyleMaskResizable | NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskClosable);
        window.titlebarAppearsTransparent = YES;
        window.titleVisibility = NSWindowTitleHidden;
        window.movable = NO;
        objc_setAssociatedObject(window, &preview_setup_key, @YES, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    }
    window.ignoresMouseEvents=YES;
    objc_setAssociatedObject(view, &preview_key, @YES, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    NSRect surface = NSMakeRect(NSMinX(view.bounds)+x,
        NSMinY(view.bounds)+(view.isFlipped ? y : view.bounds.size.height-y-height), width, height);
    refit_surface_rect(view, surface, 14.);
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

// Global top-left logical coordinates match GPUI window bounds. Read only;
// never posts an event, monitors another app, or moves the system pointer.
void murmur_overlay_pointer_position(double *out) {
    assert([NSThread isMainThread]);
    NSPoint point = NSEvent.mouseLocation;
    out[0] = point.x;
    out[1] = NSMaxY(NSScreen.screens.firstObject.frame) - point.y;
}

// Notification-driven geometry invalidation. No timer, display link, global
// event monitor or AX client. The Rust inbox owns coalescing and main-turn work.
@interface MurmurOverlayScreenWatch : NSObject
@property(nonatomic) void (*callback)(void *, bool);
@property(nonatomic) void *context;
@property(nonatomic, strong) NSUserDefaults *dock;
@property(nonatomic, strong) NSNotificationCenter *dockCenter;
@property(nonatomic, strong) NSDictionary *dockSnapshot;
@end
static NSArray<NSString *> *dock_keys(void) {
    return @[@"autohide", @"tilesize", @"magnification", @"largesize", @"orientation"];
}
static NSDictionary *dock_snapshot(NSUserDefaults *dock) {
    NSMutableDictionary *values = [NSMutableDictionary new];
    for (NSString *key in dock_keys()) values[key] = [dock objectForKey:key] ?: NSNull.null;
    return values;
}
@implementation MurmurOverlayScreenWatch
- (void)changed:(NSNotification *)notification {
    if (!NSThread.isMainThread) {
        dispatch_async(dispatch_get_main_queue(), ^{ [self changed:notification]; });
        return;
    }
    if (self.callback) self.callback(self.context, false);
}
- (void)dockChanged:(NSNotification *)notification {
    // Defaults KVO can arrive on a writer's thread. Serialize snapshots with
    // teardown and keep queued notifications inert after the Rust inbox drops.
    if (!NSThread.isMainThread) {
        dispatch_async(dispatch_get_main_queue(), ^{ [self dockChanged:notification]; });
        return;
    }
    if (!self.callback) return;
    // Distributed delivery covers writes from System Settings/the Dock;
    // defaults KVO covers direct defaults updates. Compare the relevant values
    // so duplicate delivery and unrelated defaults never wake the overlay.
    [self.dock synchronize];
    NSDictionary *next = dock_snapshot(self.dock);
    if ([next isEqual:self.dockSnapshot]) return;
    self.dockSnapshot = next;
    self.callback(self.context, true);
}
- (void)observeValueForKeyPath:(NSString *)keyPath ofObject:(id)object
                      change:(NSDictionary *)change context:(void *)context {
    (void)keyPath; (void)object; (void)change; (void)context;
    [self dockChanged:nil];
}
@end
// Inject only the defaults source and notification center in native tests; no
// test writes or broadcasts ever reach the real com.apple.dock domain.
void *murmur_overlay_watch_environment(void (*callback)(void *, bool), void *context,
                                     NSUserDefaults *dock, NSNotificationCenter *dockCenter) {
    assert([NSThread isMainThread]);
    MurmurOverlayScreenWatch *watch = [MurmurOverlayScreenWatch new];
    watch.callback = callback;
    watch.context = context;
    watch.dock = dock;
    watch.dockCenter = dockCenter;
    watch.dockSnapshot = dock_snapshot(dock);
    for (NSString *key in dock_keys()) {
        [dock addObserver:watch forKeyPath:key options:0 context:NULL];
    }
    [dockCenter addObserver:watch selector:@selector(dockChanged:)
        name:@"com.apple.dock.prefchanged" object:nil];
    for (NSNotificationName name in @[NSApplicationDidChangeScreenParametersNotification,
            NSWindowDidChangeScreenNotification]) {
        [NSNotificationCenter.defaultCenter addObserver:watch selector:@selector(changed:) name:name object:nil];
    }
    [NSWorkspace.sharedWorkspace.notificationCenter addObserver:watch selector:@selector(changed:)
        name:NSWorkspaceActiveSpaceDidChangeNotification object:nil];
    return (__bridge_retained void *)watch;
}
void *murmur_overlay_watch_screen(void (*callback)(void *, bool), void *context) {
    return murmur_overlay_watch_environment(callback, context,
        [[NSUserDefaults alloc] initWithSuiteName:@"com.apple.dock"],
        NSDistributedNotificationCenter.defaultCenter);
}
void murmur_overlay_unwatch_screen(void *raw) {
    assert([NSThread isMainThread]);
    MurmurOverlayScreenWatch *watch = (__bridge_transfer MurmurOverlayScreenWatch *)raw;
    watch.callback = NULL;
    watch.context = NULL;
    for (NSString *key in dock_keys()) [watch.dock removeObserver:watch forKeyPath:key];
    [watch.dockCenter removeObserver:watch];
    [NSNotificationCenter.defaultCenter removeObserver:watch];
    [NSWorkspace.sharedWorkspace.notificationCenter removeObserver:watch];
}

void murmur_overlay_pointer_enabled(void *raw, bool enabled) {
    assert([NSThread isMainThread]);
    NSView *view = (__bridge NSView *)raw;
    BOOL ignores = !enabled;
    if (view.window.ignoresMouseEvents != ignores) view.window.ignoresMouseEvents = ignores;
}
void murmur_overlay_material_opacity(void *raw, double opacity) {
    assert([NSThread isMainThread]);
    NSView *view = (__bridge NSView *)raw;
    MurmurOverlayMaterial *hud = objc_getAssociatedObject(view, &hud_key);
    if (hud && hud.alphaValue != opacity) hud.alphaValue = opacity;
}
