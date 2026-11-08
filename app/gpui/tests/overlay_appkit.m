// Real, own-process AppKit regression. Synthetic panel only: no capture,
// pasteboard, permissions, production profile, or application activation.
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <assert.h>
#include <stdbool.h>

@interface OverlayTestPanel : NSPanel
@property(nonatomic) NSUInteger frontOrders;
@end
@implementation OverlayTestPanel
- (void)orderFrontRegardless { self.frontOrders++; [super orderFrontRegardless]; }
@end

extern bool murmur_overlay_apply(void *, double, double, double, double,
    bool, bool, bool, bool, bool, double, double);

int main(void) {
    @autoreleasepool {
        NSApplication *app = NSApplication.sharedApplication;
        [app setActivationPolicy:NSApplicationActivationPolicyAccessory];
        assert(!app.isActive);
        OverlayTestPanel *panel = [[OverlayTestPanel alloc]
            initWithContentRect:NSMakeRect(0, 0, 300, 60)
            styleMask:NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel
            backing:NSBackingStoreBuffered defer:NO];
        panel.releasedWhenClosed = NO;
        Class original = object_getClass(panel);
        // Equivalent to GPUI's show:false, focus:false popup creation.
        assert(!panel.isVisible && !panel.isKeyWindow && !panel.isMainWindow);
        void *view = (__bridge void *)panel.contentView;
        double top = NSMaxY(NSScreen.screens.firstObject.frame);
        assert(murmur_overlay_apply(view, 80, 12, 220, 42, false, true, false, false, true, 14, 42));
        assert(!panel.isVisible && panel.frontOrders == 0);
        assert(panel.alphaValue == 0 && panel.ignoresMouseEvents);

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
        assert(!panel.hidesOnDeactivate && !panel.ignoresMouseEvents);
        assert(NSEqualRects(panel.frame, NSMakeRect(80, top-12-42, 220, 42)));

        // Repeated visible applies do not reorder; a hidden surface remains
        // a persistent, transparent, click-through window rather than ordering out.
        assert(murmur_overlay_apply(view, 80, 12, 220, 42, false, false, false, false, true, 14, 42));
        assert(panel.frontOrders == 1);
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
        [panel orderOut:nil];
        // The production companion lives until process exit. Restore the test
        // panel's AppKit/KVO class before explicitly destroying this fixture.
        object_setClass(panel, original);
        [panel close];
    }
    return 0;
}
