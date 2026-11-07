#import <AppKit/AppKit.h>
#import <objc/runtime.h>
static IMP originalAppFocus;
static id appFocusedElement(NSApplication *app, SEL cmd) {
    id focused=[app.keyWindow.contentView accessibilityFocusedUIElement];
    return focused ?: ((id(*)(id,SEL))originalAppFocus)(app,cmd);
}
// Route both native window kinds to AccessKit's content-view adapter.
static id focusedElement(NSWindow *window, SEL cmd) {
    (void)cmd;
    id focused = [window.contentView accessibilityFocusedUIElement];
    return focused ?: window;
}
void murmur_install_focus_hook(void) {
    static BOOL installed=NO;
    if (installed) return;
    installed=YES;
    for (NSString *name in @[@"GPUIWindow", @"GPUIPanel"]) {
        Class cls = NSClassFromString(name);
        if (!cls) continue;
        class_replaceMethod(cls, @selector(accessibilityFocusedUIElement), (IMP)focusedElement, "@@:");
        // NSWindow's AX bridge does not publish this informal selector merely
        // because it exists. Keep AppKit's attribute list/values and extend the
        // legacy lookup used by AXUIElementCopyAttributeValue on an AXWindow.
        SEL namesSelector = @selector(accessibilityAttributeNames);
        IMP originalNames = class_getMethodImplementation(cls, namesSelector);
        class_replaceMethod(cls, namesSelector, imp_implementationWithBlock(^NSArray *(id window) {
            NSArray *names = ((id(*)(id,SEL))originalNames)(window, namesSelector);
            if ([names containsObject:NSAccessibilityFocusedUIElementAttribute]) return names;
            return [(names ?: @[]) arrayByAddingObject:NSAccessibilityFocusedUIElementAttribute];
        }), "@@:");
        SEL valueSelector = @selector(accessibilityAttributeValue:);
        IMP originalValue = class_getMethodImplementation(cls, valueSelector);
        class_replaceMethod(cls, valueSelector, imp_implementationWithBlock(^id(id window, NSString *attribute) {
            if ([attribute isEqualToString:NSAccessibilityFocusedUIElementAttribute]) {
                return focusedElement(window, @selector(accessibilityFocusedUIElement));
            }
            return ((id(*)(id,SEL,id))originalValue)(window, valueSelector, attribute);
        }), "@@:@");
    }
    Class app=NSClassFromString(@"GPUIApplication");
    originalAppFocus=class_getMethodImplementation(app,@selector(accessibilityFocusedUIElement));
    class_addMethod(app,@selector(accessibilityFocusedUIElement),(IMP)appFocusedElement,"@@:");
}
