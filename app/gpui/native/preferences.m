#import <AppKit/AppKit.h>
// Read on GPUI's main thread. Re-read before every render, so changes take
// effect in an already-open window without owning or changing system settings.
unsigned murmur_display_preferences(void) {
    NSWorkspace *workspace = NSWorkspace.sharedWorkspace;
    return (workspace.accessibilityDisplayShouldReduceMotion ? 1 : 0)
        | (workspace.accessibilityDisplayShouldIncreaseContrast ? 2 : 0)
        | (workspace.accessibilityDisplayShouldReduceTransparency ? 4 : 0);
}
