// Real, own-process AppKit regression. Synthetic panel only: no capture,
// pasteboard, permissions, production profile, or application activation.
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <assert.h>
#include <stdbool.h>

@interface OverlayTestPanel : NSPanel
@property(nonatomic) NSUInteger frontOrders;
@property(nonatomic) NSRect expectedFirstFrame;
@property(nonatomic) NSUInteger frameWrites;
@property(nonatomic) NSUInteger synchronousPaints;
@end
@implementation OverlayTestPanel
- (void)orderFrontRegardless {
    // Assert at presentation time, not merely after the apply has returned.
    assert(NSEqualRects(self.frame, self.expectedFirstFrame));
    self.frontOrders++;
    [super orderFrontRegardless];
}
- (void)setFrame:(NSRect)frame display:(BOOL)display {
    self.frameWrites++;
    if (display) self.synchronousPaints++;
    [super setFrame:frame display:display];
}
@end

extern void *murmur_overlay_material(void *);
extern bool murmur_overlay_apply(void *, double, double, double, double,
    bool, bool, bool, bool, bool, double, double);

int main(void) {
    @autoreleasepool {
        NSApplication *app = NSApplication.sharedApplication;
        [app setActivationPolicy:NSApplicationActivationPolicyAccessory];
        assert(!app.isActive);
        OverlayTestPanel *panel = [[OverlayTestPanel alloc]
            initWithContentRect:NSMakeRect(0, 0, 300, 60)
            styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskFullSizeContentView | NSWindowStyleMaskNonactivatingPanel
            backing:NSBackingStoreBuffered defer:NO];
        panel.releasedWhenClosed = NO;
        Class original;
        // Equivalent to GPUI's show:false, focus:false popup creation.
        assert(!panel.isVisible && !panel.isKeyWindow && !panel.isMainWindow);
        void *view = (__bridge void *)panel.contentView;
        double top = NSMaxY(NSScreen.screens.firstObject.frame);
        panel.expectedFirstFrame = NSMakeRect(80, top-12-42, 220, 42);
        panel.frameWrites = panel.synchronousPaints = 0;
        assert(murmur_overlay_apply(view, 80, 12, 220, 42, false, true, false, false, true, 14, 42));
        original = class_getSuperclass(object_getClass(panel));
        assert(!panel.isVisible && panel.frontOrders == 0);
        assert(panel.alphaValue == 0 && panel.ignoresMouseEvents);
        // AppKit can resize while removing its titlebar, still ordered out.
        // Subsequent presentation and identical applies must not repeat it.
        panel.frameWrites = panel.synchronousPaints = 0;

        assert(murmur_overlay_apply(view, 80, 12, 220, 42, false, false, false, false, true, 14, 42));
        assert(panel.isVisible && panel.frontOrders == 1 && panel.alphaValue == 1);
        assert(!panel.canBecomeKeyWindow && !panel.canBecomeMainWindow);
        assert(!panel.isKeyWindow && !panel.isMainWindow && !app.isActive);
        assert(panel.level == 25);
        NSWindowCollectionBehavior expected = NSWindowCollectionBehaviorCanJoinAllSpaces |
            NSWindowCollectionBehaviorStationary | NSWindowCollectionBehaviorIgnoresCycle |
            NSWindowCollectionBehaviorFullScreenAuxiliary | NSWindowCollectionBehaviorCanJoinAllApplications;
        assert(panel.collectionBehavior == expected);
        assert(panel.styleMask & NSWindowStyleMaskNonactivatingPanel);
        assert(!(panel.styleMask & NSWindowStyleMaskTitled));
        assert(panel.styleMask & NSWindowStyleMaskFullSizeContentView);
        assert(!panel.hidesOnDeactivate && !panel.ignoresMouseEvents);
        assert(NSEqualRects(panel.frame, NSMakeRect(80, top-12-42, 220, 42)));

        // Repeated visible applies do not reorder; a hidden surface remains
        // a persistent, transparent, click-through window rather than ordering out.
        assert(murmur_overlay_apply(view, 80, 12, 220, 42, false, false, false, false, true, 14, 42));
        assert(panel.frontOrders == 1);
        assert(panel.frameWrites == 0 && panel.synchronousPaints == 0);
        assert(murmur_overlay_apply(view, 80, 12, 220, 42, false, true, false, false, true, 14, 42));
        assert(panel.isVisible && panel.alphaValue == 0 && panel.ignoresMouseEvents);
        assert(panel.frontOrders == 1 && !app.isActive);
        assert(murmur_overlay_apply(view, 80, 12, 220, 42, false, false, false, false, true, 14, 42));
        assert(panel.isVisible && panel.alphaValue == 1 && !panel.ignoresMouseEvents);
        assert(panel.frontOrders == 1 && !app.isActive);

        // Recover if the host/window server ordered it out in the meantime.
        [panel orderOut:nil];
        assert(!panel.isVisible);
        assert(murmur_overlay_apply(view, 80, 12, 220, 42, false, false, false, false, true, 14, 42));
        assert(panel.isVisible && panel.frontOrders == 2);
        assert(!panel.isKeyWindow && !panel.isMainWindow && !app.isActive);
        // Idle floating panels must not retain an active backdrop pass.
        assert(murmur_overlay_apply(view, 80, 12, 220, 42, true, true, false, false, true, 17, 34));
        NSVisualEffectView *hud = (__bridge NSVisualEffectView *)murmur_overlay_material(view);
        assert(hud.hidden && panel.ignoresMouseEvents && panel.alphaValue == 0);
        assert(murmur_overlay_apply(view, 80, 12, 220, 42, true, false, false, false, true, 17, 34));
        assert(!hud.hidden);
        assert([hud.appearance.name isEqualToString:NSAppearanceNameDarkAqua]);
        assert(murmur_overlay_apply(view, 80, 12, 220, 42, true, false, false, false, false, 17, 34));
        assert([hud.appearance.name isEqualToString:NSAppearanceNameAqua]);
        assert(murmur_overlay_apply(view, 80, 12, 220, 42, true, false, false, false, true, 17, 34));
        assert([hud.appearance.name isEqualToString:NSAppearanceNameDarkAqua]);
        assert(murmur_overlay_apply(view, 80, 12, 220, 42, true, false, true, false, true, 17, 34));
        assert(hud.hidden);
        [panel orderOut:nil];
        // The production companion lives until process exit. Restore the test
        // panel's AppKit/KVO class before explicitly destroying this fixture.
        object_setClass(panel, original);
        [panel close];
    }
    return 0;
}
