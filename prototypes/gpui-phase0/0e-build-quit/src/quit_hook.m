#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#import <time.h>
#import <unistd.h>

void p0e_log(const char *event);

// Scratch hook; no Zed code copied. Keep the existing delegate and willTerminate implementation.
static BOOL pending = NO;
static BOOL drained = NO;
static IMP originalWillTerminate;
static IMP originalTerminate;
// AppKit waits in a nested loop. Do not enter that loop inside a main GCD callback:
// GPUI dispatches quit there, which otherwise prevents both its timers and our reply.
static void terminateOutsideDispatch(id self, SEL cmd, id sender) {
    p0e_log("TERMINATE_RELAY_TO_CF_RUNLOOP");
    CFRunLoopPerformBlock(CFRunLoopGetMain(), kCFRunLoopCommonModes, ^{
        p0e_log("TERMINATE_ENTER_OUTSIDE_GCD");
        ((void (*)(id, SEL, id))originalTerminate)(self, cmd, sender);
    });
    CFRunLoopWakeUp(CFRunLoopGetMain());
}
void p0e_log(const char *event) {
    struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t);
    const char *path = getenv("P0E_LOG");
    NSString *fallback = nil;
    if (!path && [NSBundle.mainBundle.bundleIdentifier isEqualToString:@"com.georgenijo.gpui0e"]) {
        fallback = [NSBundle.mainBundle.bundlePath.stringByDeletingLastPathComponent stringByAppendingPathComponent:@"quit-relaunch-child.log"];
        path = fallback.fileSystemRepresentation;
    }
    FILE *f = path ? fopen(path, "a") : stderr;
    if (!f) abort();
    fprintf(f, "wall=%.6f mono=%.6f pid=%d main=%d %s\n", NSDate.date.timeIntervalSince1970,
        t.tv_sec + t.tv_nsec / 1e9, getpid(), NSThread.isMainThread, event);
    fflush(f); if (path) fclose(f);
}
static NSApplicationTerminateReply shouldTerminate(id self, SEL cmd, NSApplication *app) {
    NSAppleEventDescriptor *event = NSAppleEventManager.sharedAppleEventManager.currentAppleEvent;
    NSString *detail = [NSString stringWithFormat:@"SHOULD_TERMINATE class=%@ appleEvent=%@ reason=%@ pending=%d",
        NSStringFromClass([self class]), event, [event paramDescriptorForKeyword:'why?'], pending];
    p0e_log([detail stringByReplacingOccurrencesOfString:@"\n" withString:@" "].UTF8String);
    if (drained) { p0e_log("RETURN_NSTerminateNow_DRAINED"); return NSTerminateNow; }
    if (!pending) {
        pending = YES;
        dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
            p0e_log("TEARDOWN_START background");
            usleep(6000000);
            p0e_log("TEARDOWN_END background");
            dispatch_async(dispatch_get_main_queue(), ^{
                drained = YES;
                p0e_log("REPLY_TRUE main_dispatch");
                [app replyToApplicationShouldTerminate:YES];
            });
        });
    }
    p0e_log("RETURN_NSTerminateLater");
    return NSTerminateLater;
}
static void willTerminate(id self, SEL cmd, NSNotification *note) {
    p0e_log("WILL_TERMINATE_BEFORE_GPUI");
    ((void (*)(id, SEL, id))originalWillTerminate)(self, cmd, note);
    p0e_log("WILL_TERMINATE_AFTER_GPUI");
}
void p0e_install(void) {
    NSCAssert(NSThread.isMainThread, @"main thread only");
    Class cls = [NSApp.delegate class];
    NSCAssert([NSStringFromClass(cls) isEqualToString:@"GPUIApplicationDelegate"], @"unexpected delegate");
    NSCAssert(class_addMethod(cls, @selector(applicationShouldTerminate:), (IMP)shouldTerminate, "Q@:@"), @"hook already present");
    Method m = class_getInstanceMethod(cls, @selector(applicationWillTerminate:));
    NSCAssert(m, @"missing original method");
    originalWillTerminate = method_setImplementation(m, (IMP)willTerminate);
    if (!getenv("P0E_STOCK_TERMINATE")) {
        Class appClass = [NSApp class];
        originalTerminate = class_getMethodImplementation(appClass, @selector(terminate:));
        NSCAssert(class_addMethod(appClass, @selector(terminate:), (IMP)terminateOutsideDispatch, "v@:@"), @"terminate relay already present");
    }
    p0e_log("HOOK_INSTALLED GPUIApplicationDelegate plus GPUIApplication terminate relay");
}
