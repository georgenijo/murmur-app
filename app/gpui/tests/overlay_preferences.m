// Windowless notification/KVO regression. An in-memory defaults subclass and
// private notification center never write/broadcast real Dock preferences.
#import <AppKit/AppKit.h>
#include <assert.h>
#include <stdbool.h>

@interface SyntheticDockDefaults : NSUserDefaults
@property(nonatomic, strong) NSMutableDictionary *values;
@end
@implementation SyntheticDockDefaults
- (id)objectForKey:(NSString *)key { return self.values[key]; }
- (BOOL)synchronize { return YES; }
@end

extern void *murmur_overlay_watch_environment(void (*)(void *, bool), void *, NSUserDefaults *, NSNotificationCenter *);
extern void murmur_overlay_unwatch_screen(void *);
static void changed(void *context, bool dock) {
    assert(NSThread.isMainThread);
    if (dock) (*(unsigned *)context)++;
}
int main(void) {
    @autoreleasepool {
        SyntheticDockDefaults *defaults = [SyntheticDockDefaults new];
        defaults.values = [@{@"autohide": @NO, @"tilesize": @64,
            @"magnification": @NO, @"largesize": @128, @"orientation": @"bottom"} mutableCopy];
        NSNotificationCenter *center = [NSNotificationCenter new];
        unsigned count = 0;
        void *watch = murmur_overlay_watch_environment(changed, &count, defaults, center);
        [NSRunLoop.currentRunLoop runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
        assert(count == 0); // Installing and idling never invalidate.
        [center postNotificationName:@"com.apple.dock.prefchanged" object:nil];
        assert(count == 0); // Duplicate/irrelevant defaults changes are inert.
        NSDictionary *next = @{@"autohide": @YES, @"tilesize": @96,
            @"magnification": @YES, @"largesize": @160, @"orientation": @"left"};
        for (NSString *key in next) {
            unsigned before = count;
            // Simulate external cfprefsd/defaults KVO delivery, not a write to
            // the actual Dock suite. Each relevant preference invalidates once.
            [defaults willChangeValueForKey:key];
            defaults.values[key] = next[key];
            [defaults didChangeValueForKey:key];
            assert(count == before + 1);
            [center postNotificationName:@"com.apple.dock.prefchanged" object:nil];
            assert(count == before + 1); // KVO + distributed delivery coalesce.
        }
        // External writers can arrive solely via the distributed notification.
        defaults.values[@"orientation"] = @"right";
        [center postNotificationName:@"com.apple.dock.prefchanged" object:nil];
        assert(count == 6);
        [center postNotificationName:@"com.apple.dock.prefchanged" object:nil];
        assert(count == 6);
        // Delivery on a writer thread is serialized onto main, without a timer.
        defaults.values[@"tilesize"] = @42;
        dispatch_group_t group = dispatch_group_create();
        dispatch_group_async(group, dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{
            [center postNotificationName:@"com.apple.dock.prefchanged" object:nil];
        });
        dispatch_group_wait(group, DISPATCH_TIME_FOREVER);
        assert(count == 6);
        [NSRunLoop.currentRunLoop runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
        assert(count == 7);
        // A queued delivery must not call the borrowed Rust context after drop.
        defaults.values[@"tilesize"] = @43;
        dispatch_group_async(group, dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{
            [center postNotificationName:@"com.apple.dock.prefchanged" object:nil];
        });
        dispatch_group_wait(group, DISPATCH_TIME_FOREVER);
        murmur_overlay_unwatch_screen(watch);
        [NSRunLoop.currentRunLoop runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
        [defaults willChangeValueForKey:@"tilesize"];
        defaults.values[@"tilesize"] = @32;
        [defaults didChangeValueForKey:@"tilesize"];
        [center postNotificationName:@"com.apple.dock.prefchanged" object:nil];
        assert(count == 7);
    }
    return 0;
}
