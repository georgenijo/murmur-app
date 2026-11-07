#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <stdbool.h>
// Add the missing selector to GPUI's existing delegate. Never swizzle or
// replace its delegate; reject a backend that already owns this selector.
static NSInteger (*termination)(void *);
static void *terminationContext;
static BOOL pending;
static NSApplicationTerminateReply murmurShouldTerminate(id self, SEL cmd, NSApplication *app) {
    (void)self; (void)cmd; (void)app;
    NSCAssert([NSThread isMainThread], @"termination admission requires main");
    NSInteger reply = termination ? termination(terminationContext) : NSTerminateNow;
    pending = reply == NSTerminateLater;
    return reply;
}
bool murmur_host_install_termination(NSInteger (*callback)(void *), void *context) {
    if (![NSThread isMainThread] || !NSApp.delegate || termination) return false;
    Class delegateClass = [NSApp.delegate class];
    if (class_getInstanceMethod(delegateClass, @selector(applicationShouldTerminate:))) return false;
    if (!class_addMethod(delegateClass, @selector(applicationShouldTerminate:),
                         (IMP)murmurShouldTerminate, "q@:@")) return false;
    termination = callback;
    terminationContext = context;
    return true;
}
void murmur_host_remove_termination(void *context) {
    NSCAssert([NSThread isMainThread], @"termination removal requires main");
    if (terminationContext == context) { termination = NULL; terminationContext = NULL; }
}
bool murmur_host_reply_termination(void) {
    NSCAssert([NSThread isMainThread], @"termination reply requires main");
    if (!pending) return false;
    pending = NO;
    // Leave GPUI's AppCell borrow before its will-terminate callback.
    dispatch_async(dispatch_get_main_queue(), ^{ [NSApp replyToApplicationShouldTerminate:YES]; });
    return true;
}
@interface MurmurHostTarget : NSObject
@property(nonatomic, assign) void (*callback)(void *, const char *);
@property(nonatomic, assign) void *context;
@property(nonatomic, weak) NSStatusItem *statusItem;
@property(nonatomic, strong) NSMenu *menu;
@end
@implementation MurmurHostTarget
- (void)perform:(NSMenuItem *)item { self.callback(self.context, [item.representedObject UTF8String]); }
- (void)statusClick:(id)sender {
    if (NSApp.currentEvent.type == NSEventTypeRightMouseUp) [self.menu popUpMenuPositioningItem:nil atLocation:NSMakePoint(0, 0) inView:self.statusItem.button];
    else self.callback(self.context, "show");
}
@end
void *murmur_host_target(void (*callback)(void *, const char *), void *context) {
    NSCAssert([NSThread isMainThread], @"tray target requires main");
    MurmurHostTarget *target = [MurmurHostTarget new];
    target.callback = callback; target.context = context;
    return (__bridge_retained void *)target;
}
static NSMenu *buildMenu(NSArray *items, id target) {
    NSMenu *menu = [NSMenu new];
    menu.autoenablesItems = NO;
    for (NSDictionary *item in items) {
        if ([item[@"separator"] boolValue]) { [menu addItem:NSMenuItem.separatorItem]; continue; }
        NSMenuItem *entry = [[NSMenuItem alloc] initWithTitle:item[@"title"] ?: @"" action:@selector(perform:) keyEquivalent:@""];
        entry.target = target;
        entry.representedObject = item[@"id"] ?: @"";
        entry.enabled = ![item[@"disabled"] boolValue];
        entry.state = [item[@"checked"] boolValue] ? NSControlStateValueOn : NSControlStateValueOff;
        if (item[@"items"]) entry.submenu = buildMenu(item[@"items"], target);
        [menu addItem:entry];
    }
    return menu;
}
void *murmur_host_menu(const char *json, void *target) {
    if (![NSThread isMainThread] || !json || !target) return NULL;
    NSData *data = [[NSString stringWithUTF8String:json] dataUsingEncoding:NSUTF8StringEncoding];
    id items = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
    if (![items isKindOfClass:NSArray.class]) return NULL;
    return (__bridge_retained void *)buildMenu(items, (__bridge id)target);
}
void murmur_host_window_visible(void *view, bool visible) {
    NSCAssert([NSThread isMainThread], @"window mutation requires main");
    NSWindow *window = [(__bridge NSView *)view window];
    if (visible) { [window makeKeyAndOrderFront:nil]; [NSApp activateIgnoringOtherApps:YES]; }
    else { [window orderOut:nil]; }
}
bool murmur_host_window_is_visible(void *view) {
    NSCAssert([NSThread isMainThread], @"window query requires main");
    return [(__bridge NSView *)view window].visible;
}

void murmur_host_attach_tray(void *itemPointer, void *menuPointer, void *targetPointer) {
    NSCAssert([NSThread isMainThread], @"tray mutation requires main");
    NSStatusItem *item = (__bridge NSStatusItem *)itemPointer;
    MurmurHostTarget *target = (__bridge MurmurHostTarget *)targetPointer;
    target.statusItem = item;
    target.menu = (__bridge NSMenu *)menuPointer;
    item.button.target = target;
    item.button.action = @selector(statusClick:);
    [item.button sendActionOn:NSEventMaskLeftMouseUp | NSEventMaskRightMouseUp];
}
