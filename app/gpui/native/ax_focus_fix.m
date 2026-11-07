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
        if (cls) class_addMethod(cls, @selector(accessibilityFocusedUIElement), (IMP)focusedElement, "@@:");
    }
    Class app=NSClassFromString(@"GPUIApplication");
    originalAppFocus=class_getMethodImplementation(app,@selector(accessibilityFocusedUIElement));
    class_addMethod(app,@selector(accessibilityFocusedUIElement),(IMP)appFocusedElement,"@@:");
}
