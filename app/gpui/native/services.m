#import <AppKit/AppKit.h>
#import <UserNotifications/UserNotifications.h>
#import <ServiceManagement/ServiceManagement.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>

typedef void (*MurmurReply)(void *, const char *);
typedef bool (*MurmurLive)(void *);
static void replyJSON(MurmurReply reply, void *context, id value) {
    NSData *data = [NSJSONSerialization dataWithJSONObject:value options:NSJSONWritingFragmentsAllowed error:nil];
    NSString *text = data ? [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] : @"null";
    reply(context, text.UTF8String);
}
static NSDictionary *requestJSON(const char *text) {
    if (!text) return nil;
    NSData *data = [[NSString stringWithUTF8String:text] dataUsingEncoding:NSUTF8StringEncoding];
    id value = data ? [NSJSONSerialization JSONObjectWithData:data options:0 error:nil] : nil;
    return [value isKindOfClass:NSDictionary.class] ? value : nil;
}
static bool installedIdentity(void) {
    NSBundle *bundle = NSBundle.mainBundle;
    NSString *identifier = bundle.bundleIdentifier;
    return identifier.length && [bundle.bundleURL.pathExtension isEqualToString:@"app"];
}
/// No AppKit object is constructed on the worker-mode path. Foundation's bundle
/// metadata is safe to query before GPUI/NSApplication/Metal initialization.
char *murmur_services_identity(void) {
    NSString *identifier = NSBundle.mainBundle.bundleIdentifier;
    return strdup((identifier.length ? identifier : @"com.localdictation.dev").UTF8String);
}
char *murmur_services_bundle_path(void) {
    return strdup(NSBundle.mainBundle.bundlePath.UTF8String);
}
void murmur_services_free(char *value) { free(value); }

void murmur_services_panel(const char *json, MurmurLive live, MurmurReply reply, void *context) {
    NSCAssert(NSThread.isMainThread, @"panel main thread");
    NSDictionary *request = requestJSON(json);
    if (!request || !live(context)) { replyJSON(reply,context,NSNull.null); return; }
    NSString *kind=request[@"kind"];
    if ([kind isEqualToString:@"alert"]) {
        NSAlert *alert=[NSAlert new];
        alert.messageText=request[@"title"] ?: @"Murmur";
        alert.informativeText=request[@"message"] ?: @"";
        [alert addButtonWithTitle:request[@"confirm"] ?: @"OK"];
        if ([request[@"cancel"] boolValue]) [alert addButtonWithTitle:@"Cancel"];
        if (!NSApp.keyWindow) {replyJSON(reply,context,NSNull.null);return;}
        NSTimer *watch=[NSTimer timerWithTimeInterval:0.1 repeats:YES block:^(NSTimer *__unused timer){
            if (!live(context)) [NSApp endSheet:alert.window returnCode:NSModalResponseCancel];
        }];
        [NSRunLoop.mainRunLoop addTimer:watch forMode:NSRunLoopCommonModes];
        void (^done)(NSModalResponse)=^(NSModalResponse result) {
            [watch invalidate];
            replyJSON(reply,context,live(context) ? @(result==NSAlertFirstButtonReturn) : NSNull.null);
        };
        [alert beginSheetModalForWindow:NSApp.keyWindow completionHandler:done];
        return;
    }
    BOOL save=[kind isEqualToString:@"save"];
    NSSavePanel *panel=save ? [NSSavePanel savePanel] : [NSOpenPanel openPanel];
    panel.title=request[@"title"] ?: @"Murmur";
    if ([request[@"name"] isKindOfClass:NSString.class]) panel.nameFieldStringValue=request[@"name"];
    if ([request[@"extensions"] isKindOfClass:NSArray.class]) {
        NSMutableArray<UTType *> *types=[NSMutableArray new];
        for (id extension in request[@"extensions"]) {
            if ([extension isKindOfClass:NSString.class]) {UTType *type=[UTType typeWithFilenameExtension:extension];if(type)[types addObject:type];}
        }
        if(types.count)panel.allowedContentTypes=types;
    }
    if (!save) {
        NSOpenPanel *open=(NSOpenPanel *)panel;
        open.canChooseDirectories=[request[@"directory"] boolValue];
        open.canChooseFiles=!open.canChooseDirectories;
        open.allowsMultipleSelection=[request[@"multiple"] boolValue];
        open.resolvesAliases=YES;
    }
    NSTimer *watch=[NSTimer timerWithTimeInterval:0.1 repeats:YES block:^(NSTimer *__unused timer){
        if (!live(context)) [panel cancel:nil];
    }];
    [NSRunLoop.mainRunLoop addTimer:watch forMode:NSRunLoopCommonModes];
    void (^done)(NSModalResponse)=^(NSModalResponse result) {
        [watch invalidate];
        if (result!=NSModalResponseOK || !live(context)) {replyJSON(reply,context,NSNull.null);return;}
        if (save) replyJSON(reply,context,panel.URL.path ?: NSNull.null);
        else {
            NSMutableArray *paths=[NSMutableArray new];
            for (NSURL *url in ((NSOpenPanel *)panel).URLs) if (url.fileURL && url.path) [paths addObject:url.path];
            replyJSON(reply,context,paths);
        }
    };
    if (NSApp.keyWindow) [panel beginSheetModalForWindow:NSApp.keyWindow completionHandler:done];
    else [panel beginWithCompletionHandler:done];
}
bool murmur_services_open(const char *raw, bool file) {
    NSCAssert(NSThread.isMainThread,@"workspace main thread");
    NSString *text=raw ? [NSString stringWithUTF8String:raw] : nil;
    if (file && !text.isAbsolutePath) return false;
    NSURL *url=file ? [NSURL fileURLWithPath:text ?: @""] : [NSURL URLWithString:text ?: @""];
    if (!url) return false;
    if (!file && ![@[@"https",@"http",@"mailto",@"x-apple.systempreferences"] containsObject:url.scheme.lowercaseString]) return false;
    return [NSWorkspace.sharedWorkspace openURL:url];
}
int murmur_services_login_status(void) {
    NSCAssert(NSThread.isMainThread,@"login status main thread");
    if (!installedIdentity()) return -1;
    if (@available(macOS 13.0,*)) {
        switch (SMAppService.mainAppService.status) {
            case SMAppServiceStatusNotRegistered:return 0;
            case SMAppServiceStatusEnabled:return 1;
            case SMAppServiceStatusRequiresApproval:return 2;
            case SMAppServiceStatusNotFound:return -1;
        }
    }
    return -1;
}
bool murmur_services_login_set(bool enabled) {
    NSCAssert(NSThread.isMainThread,@"login registration main thread");
    if (!installedIdentity()) return false;
    if (@available(macOS 13.0,*)) {
        NSError *error=nil;
        return enabled ? [SMAppService.mainAppService registerAndReturnError:&error]
                       : [SMAppService.mainAppService unregisterAndReturnError:&error];
    }
    return false;
}
@interface MurmurNotificationDelegate : NSObject<UNUserNotificationCenterDelegate>
@property(nonatomic) void (*activate)(void);
@end
@implementation MurmurNotificationDelegate
- (void)userNotificationCenter:(UNUserNotificationCenter *)center willPresentNotification:(UNNotification *)notification withCompletionHandler:(void (^)(UNNotificationPresentationOptions))done {
    done(UNNotificationPresentationOptionBanner | UNNotificationPresentationOptionSound);
}
- (void)userNotificationCenter:(UNUserNotificationCenter *)center didReceiveNotificationResponse:(UNNotificationResponse *)response withCompletionHandler:(void (^)(void))done {
    if ([response.notification.request.identifier isEqualToString:@"murmur-update"] && self.activate) dispatch_async(dispatch_get_main_queue(),^{self.activate();});
    done();
}
@end
static MurmurNotificationDelegate *notifications;
bool murmur_services_notifications(void (*activate)(void)) {
    NSCAssert(NSThread.isMainThread,@"notification delegate main thread");
    if (!installedIdentity()) return false;
    notifications=[MurmurNotificationDelegate new];
    notifications.activate=activate;
    UNUserNotificationCenter.currentNotificationCenter.delegate=notifications;
    return true;
}
void murmur_services_notify(const char *raw, bool requestPermission) {
    NSCAssert(NSThread.isMainThread,@"notification main thread");
    if (!installedIdentity()) return;
    NSString *version=raw ? [NSString stringWithUTF8String:raw] : nil;
    if (!version.length || version.length>80) return;
    UNUserNotificationCenter *center=UNUserNotificationCenter.currentNotificationCenter;
    void (^post)(void)=^{
        UNMutableNotificationContent *content=[UNMutableNotificationContent new];
        content.title=@"Murmur update available";
        content.body=[NSString stringWithFormat:@"Murmur %@ is ready. Open Murmur to review the update.",version];
        content.sound=UNNotificationSound.defaultSound;
        [center addNotificationRequest:[UNNotificationRequest requestWithIdentifier:@"murmur-update" content:content trigger:nil] withCompletionHandler:^(NSError *__unused error) {}];
    };
    [center getNotificationSettingsWithCompletionHandler:^(UNNotificationSettings *settings){
        if (settings.authorizationStatus==UNAuthorizationStatusAuthorized || settings.authorizationStatus==UNAuthorizationStatusProvisional) post();
        else if (settings.authorizationStatus==UNAuthorizationStatusNotDetermined && requestPermission)
            [center requestAuthorizationWithOptions:UNAuthorizationOptionAlert|UNAuthorizationOptionSound completionHandler:^(BOOL granted,NSError *__unused error){if(granted)post();}];
    }];
}

@interface MurmurWindowBinding : NSObject
@property(nonatomic,weak) NSWindow *window;
@property(nonatomic) uint64_t generation;
@end
@implementation MurmurWindowBinding
@end
static NSMutableDictionary<NSString *,MurmurWindowBinding *> *windows;
static MurmurWindowBinding *binding(const char *label,uint64_t generation) {
    NSCAssert(NSThread.isMainThread,@"window registry main thread");
    MurmurWindowBinding *item=windows[label ? [NSString stringWithUTF8String:label] : @""];
    return item.generation==generation && item.window ? item : nil;
}
bool murmur_services_window_register(void *view,const char *label,uint64_t generation) {
    NSCAssert(NSThread.isMainThread,@"window registration main thread");
    NSWindow *window=[(__bridge NSView *)view window];
    if (!window||!label) return false;
    if (!windows) windows=[NSMutableDictionary new];
    MurmurWindowBinding *item=[MurmurWindowBinding new];item.window=window;item.generation=generation;
    windows[[NSString stringWithUTF8String:label]]=item;
    return true;
}
void murmur_services_window_unregister(const char *label,uint64_t generation) {
    if (binding(label,generation)) [windows removeObjectForKey:[NSString stringWithUTF8String:label]];
}
void *murmur_services_window(const char *label,uint64_t generation) {
    return (__bridge void *)binding(label,generation).window;
}
static double desktopTop(void) {return NSMaxY(NSScreen.screens.firstObject.frame);}
static NSArray *physicalRect(NSRect r,double scale) {
    return @[@(round(r.origin.x*scale)),@(round((desktopTop()-NSMaxY(r))*scale)),@(round(r.size.width*scale)),@(round(r.size.height*scale))];
}
static NSDictionary *monitor(NSScreen *screen) {
    if (!screen) return nil;
    return @{@"name":screen.localizedName ?: @"Display",@"scale":@(screen.backingScaleFactor),@"frame":physicalRect(screen.frame,screen.backingScaleFactor),@"work":physicalRect(screen.visibleFrame,screen.backingScaleFactor)};
}
char *murmur_services_displays(void) {
    NSCAssert(NSThread.isMainThread,@"display query main thread");
    NSMutableArray *all=[NSMutableArray new];
    for (NSScreen *screen in NSScreen.screens) [all addObject:monitor(screen)];
    NSDictionary *value=@{@"all":all,@"primary":monitor(NSScreen.screens.firstObject) ?: NSNull.null};
    NSData *data=[NSJSONSerialization dataWithJSONObject:value options:0 error:nil];
    return strdup([[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding].UTF8String);
}
char *murmur_services_window_snapshot(const char *label,uint64_t generation) {
    NSWindow *w=binding(label,generation).window;
    if (!w) return NULL;
    double scale=w.backingScaleFactor;
    NSRect frame=w.frame;
    NSRect content=[w convertRectToScreen:[w.contentView convertRect:w.contentView.bounds toView:nil]];
    NSDictionary *value=@{@"visible":@(w.visible),@"minimized":@(w.miniaturized),@"maximized":@(w.zoomed),@"fullscreen":@((BOOL)((w.styleMask&NSWindowStyleMaskFullScreen)!=0)),@"scale":@(scale),@"outer":physicalRect(frame,scale),@"inner":physicalRect(content,scale),@"monitor":monitor(w.screen) ?: NSNull.null};
    NSData *data=[NSJSONSerialization dataWithJSONObject:value options:0 error:nil];
    return strdup([[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding].UTF8String);
}
bool murmur_services_window_command(const char *label,uint64_t generation,const char *raw) {
    NSWindow *w=binding(label,generation).window;
    NSDictionary *c=requestJSON(raw);
    if (!w||!c) return false;
    NSString *kind=c[@"kind"];
    if ([kind isEqualToString:@"show"]) [w orderFront:nil];
    else if ([kind isEqualToString:@"hide"]) [w orderOut:nil];
    else if ([kind isEqualToString:@"focus"]) {[w makeKeyAndOrderFront:nil];[NSApp activateIgnoringOtherApps:YES];}
    else if ([kind isEqualToString:@"minimize"]) [w miniaturize:nil];
    else if ([kind isEqualToString:@"unminimize"]) [w deminiaturize:nil];
    else if ([kind isEqualToString:@"maximize"]) {if(!w.zoomed)[w zoom:nil];}
    else if ([kind isEqualToString:@"fullscreen"]) {if(((w.styleMask&NSWindowStyleMaskFullScreen)!=0)!=[c[@"value"] boolValue])[w toggleFullScreen:nil];}
    else if ([kind isEqualToString:@"center"]) [w center];
    else if ([kind isEqualToString:@"cursor"]) w.ignoresMouseEvents=[c[@"value"] boolValue];
    else if ([kind isEqualToString:@"focusable"]) {if(![c[@"value"] boolValue]){w.styleMask|=NSWindowStyleMaskNonactivatingPanel;w.hidesOnDeactivate=NO;}}
    else if ([kind isEqualToString:@"background"]) {
        NSArray *rgba=c[@"rgba"];
        w.backgroundColor=rgba.count==4 ? [NSColor colorWithSRGBRed:[rgba[0] doubleValue]/255. green:[rgba[1] doubleValue]/255. blue:[rgba[2] doubleValue]/255. alpha:[rgba[3] doubleValue]/255.] : NSColor.windowBackgroundColor;
    }
    else if ([kind isEqualToString:@"size"]) {
        double factor=[c[@"physical"] boolValue] ? w.backingScaleFactor : 1.;
        double width=[c[@"x"] doubleValue]/factor,height=[c[@"y"] doubleValue]/factor;
        if (!isfinite(width)||!isfinite(height)||width<=0||height<=0||width>32768||height>32768)return false;
        [w setContentSize:NSMakeSize(width,height)];
    }
    else if ([kind isEqualToString:@"position"]) {
        double factor=[c[@"physical"] boolValue] ? w.backingScaleFactor : 1.;
        double x=[c[@"x"] doubleValue]/factor,y=[c[@"y"] doubleValue]/factor;
        if (!isfinite(x)||!isfinite(y)||fabs(x)>100000||fabs(y)>100000)return false;
        [w setFrameTopLeftPoint:NSMakePoint(x,desktopTop()-y)];
    }
    else return false;
    return true;
}
