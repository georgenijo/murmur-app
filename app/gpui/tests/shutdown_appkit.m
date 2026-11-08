// Windowless synthetic counterpart of the former Tao shutdown_appkit fixture.
// Link the production GPUI host bridge, with no app profile, capture or input.
#import <AppKit/AppKit.h>
#include <assert.h>
#include <stdbool.h>
#include <unistd.h>
#include <stdio.h>

extern bool murmur_host_install_termination(NSInteger (*callback)(void *), void *context);
extern bool murmur_host_reply_termination(void);

static BOOL entered;
static BOOL replied;
static NSUInteger callbacks;
static CFAbsoluteTime started;

static void enqueueMain(void (^block)(void)) {
    CFRunLoopPerformBlock(CFRunLoopGetMain(), kCFRunLoopCommonModes, block);
    CFRunLoopWakeUp(CFRunLoopGetMain());
}

static NSInteger admission(void *context) {
    assert(NSThread.isMainThread);
    assert(context == &entered);
    assert(!entered);
    entered = YES;
    started = CFAbsoluteTimeGetCurrent();
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{
        for (NSUInteger i = 0; i < 65; ++i) {
            usleep(100000);
            enqueueMain(^{
                assert(NSThread.isMainThread);
                ++callbacks;
            });
        }
        enqueueMain(^{
            assert(callbacks == 65);
            assert(CFAbsoluteTimeGetCurrent() - started >= 6.0);
            replied = YES;
            assert(murmur_host_reply_termination());
            assert(!murmur_host_reply_termination());
        });
    });
    return NSTerminateLater;
}

@interface ShutdownDelegate : NSObject <NSApplicationDelegate>
@end
@implementation ShutdownDelegate
- (void)applicationWillTerminate:(NSNotification *)notification {
    (void)notification;
    assert(NSThread.isMainThread);
    assert(entered && replied && callbacks == 65);
    fprintf(stderr, "GPUI host: deferred AppKit termination drained 65 main-thread callbacks before exit\n");
}
@end

int main(void) {
    @autoreleasepool {
        // A lost termination reply or callbacks must fail rather than hang CI.
        alarm(20);
        NSApplication *app = NSApplication.sharedApplication;
        [app setActivationPolicy:NSApplicationActivationPolicyProhibited];
        ShutdownDelegate *delegate = [ShutdownDelegate new];
        app.delegate = delegate;
        assert(!murmur_host_reply_termination());
        assert(murmur_host_install_termination(admission, &entered));
        assert(!murmur_host_install_termination(admission, &entered));
        [app finishLaunching];
        enqueueMain(^{ [NSApp terminate:nil]; });
        [app run];
        // Successful termination exits via AppKit, never by stopping the loop.
        return 1;
    }
}
