#import <AppKit/AppKit.h>
#import <objc/runtime.h>
static IMP originalAppFocus;
static id appFocusedElement(NSApplication *app, SEL cmd) {
    id focused=[app.keyWindow.contentView accessibilityFocusedUIElement];
    return focused ?: ((id(*)(id,SEL))originalAppFocus)(app,cmd);
}
// Production candidate: route window focus to AccessKit's content-view adapter.
static id focusedElement(NSWindow *window, SEL cmd) {
    (void)cmd;
    id focused = [window.contentView accessibilityFocusedUIElement];
    return focused ?: window;
}
void murmur_install_focus_hook(void) {
    static BOOL installed=NO;
    if (installed) return;
    installed=YES;
    Class cls = NSClassFromString(@"GPUIWindow");
    class_addMethod(cls, @selector(accessibilityFocusedUIElement), (IMP)focusedElement, "@@:");
    Class app=NSClassFromString(@"GPUIApplication");
    originalAppFocus=class_getMethodImplementation(app,@selector(accessibilityFocusedUIElement));
    class_addMethod(app,@selector(accessibilityFocusedUIElement),(IMP)appFocusedElement,"@@:");
}
