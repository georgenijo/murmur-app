// WebKit exists only in the disposable migration worker. This translation unit
// has no WebKit class references or framework link; the GPUI host never loads it.
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#import <WebKit/WKURLSchemeHandler.h>
#import <WebKit/WKScriptMessageHandler.h>
#import <WebKit/WKNavigationDelegate.h>
#import <dlfcn.h>
@protocol MurmurLegacyAPI
+ (id)defaultDataStore;
+ (id)nonPersistentDataStore;
- (id)initWithFrame:(NSRect)frame configuration:(id)configuration;
- (void)setWebsiteDataStore:(id)store;
- (id)websiteDataStore;
- (void)setURLSchemeHandler:(id)handler forURLScheme:(NSString *)scheme;
- (id)userContentController;
- (void)addScriptMessageHandler:(id)handler name:(NSString *)name;
- (void)removeScriptMessageHandlerForName:(NSString *)name;
- (void)setNavigationDelegate:(id)delegate;
- (void)loadRequest:(NSURLRequest *)request;
- (void)stopLoading;
- (void)evaluateJavaScript:(NSString *)script completionHandler:(void(^)(id,NSError *))completion;
- (NSURLRequest *)request;
- (id)body;
- (void)didReceiveResponse:(NSURLResponse *)response;
- (void)didReceiveData:(NSData *)data;
- (void)didFinish;
@end
@interface MurmurLegacyBridge : NSObject <WKURLSchemeHandler,WKScriptMessageHandler,WKNavigationDelegate>
@property(nonatomic,strong) id<MurmurLegacyAPI> view;
@property(nonatomic,strong) id<MurmurLegacyAPI> configuration;
@property(nonatomic,strong) NSMutableArray<NSString *> *messages;
@property(nonatomic,strong) NSData *page;
@property(nonatomic) BOOL failed;
@end
static int stage=10;
@implementation MurmurLegacyBridge
- (void)webView:(id)view startURLSchemeTask:(id<MurmurLegacyAPI>)task {
    stage=18; NSURL *url=task.request.URL;
    if (![url.absoluteString isEqualToString:@"tauri://localhost/"]) {self.failed=YES;return;}
    NSURLResponse *response=[[NSURLResponse alloc] initWithURL:url MIMEType:@"text/html" expectedContentLength:self.page.length textEncodingName:@"utf-8"];
    [task didReceiveResponse:response];[task didReceiveData:self.page];[task didFinish];
}
- (void)webView:(id)view stopURLSchemeTask:(id)task {}
- (void)userContentController:(id)controller didReceiveScriptMessage:(id<MurmurLegacyAPI>)message {
    stage=19; id body=message.body;
    if (![body isKindOfClass:NSString.class] || [(NSString *)body length]>32*1024*1024 || self.messages.count>=2) {self.failed=YES;return;}
    [self.messages addObject:body];
}
- (void)webView:(id)view decidePolicyForNavigationAction:(id<MurmurLegacyAPI>)action decisionHandler:(void(^)(WKNavigationActionPolicy))done {
    done([action.request.URL.absoluteString isEqualToString:@"tauri://localhost/"] ? 1 : 0);
}
- (void)webView:(id)view didFailProvisionalNavigation:(id)navigation withError:(NSError *)error {self.failed=YES;}
- (void)webView:(id)view didFailNavigation:(id)navigation withError:(NSError *)error {self.failed=YES;}
- (void)webViewWebContentProcessDidTerminate:(id)view {self.failed=YES;}
@end
static MurmurLegacyBridge *legacy;
int murmur_legacy_stage(void){return stage;}
bool murmur_legacy_start(const char *page, bool synthetic) {
    if (!NSThread.isMainThread || legacy || !page) return false;
    stage=11; NSBundle *bundle=NSBundle.mainBundle;
    if (synthetic) {
        if (![bundle.bundleIdentifier isEqualToString:@"com.localdictation.gpui-host-qa110"]) return false;
    } else if (!bundle.bundleIdentifier.length || ![bundle.bundleURL.pathExtension isEqualToString:@"app"]) return false;
    stage=12; if (!dlopen("/System/Library/Frameworks/WebKit.framework/WebKit",RTLD_NOW|RTLD_LOCAL)) return false;
    stage=14; [NSApplication sharedApplication];[NSApp setActivationPolicy:NSApplicationActivationPolicyProhibited];
    Class storeClass=NSClassFromString(@"WKWebsiteDataStore"), configClass=NSClassFromString(@"WKWebViewConfiguration"),viewClass=NSClassFromString(@"WKWebView");
    stage=15; if(!storeClass||!configClass||!viewClass)return false;
    legacy=[MurmurLegacyBridge new];legacy.messages=[NSMutableArray new];
    legacy.page=[[NSString stringWithUTF8String:page] dataUsingEncoding:NSUTF8StringEncoding];
    id<MurmurLegacyAPI> store=synthetic ? [(id<MurmurLegacyAPI>)storeClass nonPersistentDataStore] : [(id<MurmurLegacyAPI>)storeClass defaultDataStore];
    legacy.configuration=(id<MurmurLegacyAPI>)[configClass new];
    [legacy.configuration setWebsiteDataStore:store];
    [legacy.configuration setURLSchemeHandler:legacy forURLScheme:@"tauri"];
    [(id<MurmurLegacyAPI>)legacy.configuration.userContentController addScriptMessageHandler:legacy name:@"migration"];
    legacy.view=[(id<MurmurLegacyAPI>)[viewClass alloc] initWithFrame:NSZeroRect configuration:legacy.configuration];
    stage=16; if(!legacy.view || legacy.configuration.websiteDataStore != store) {legacy.failed=YES;return false;}
    [legacy.view setNavigationDelegate:legacy];
    stage=17; [legacy.view loadRequest:[NSURLRequest requestWithURL:[NSURL URLWithString:@"tauri://localhost/"]]];
    return true;
}
void murmur_legacy_pump(void) {
    @autoreleasepool { [NSRunLoop.currentRunLoop runMode:NSDefaultRunLoopMode beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.01]]; }
}
bool murmur_legacy_failed(void) {return !legacy || legacy.failed;}
char *murmur_legacy_message(void) {
    if (!legacy.messages.count) return NULL;
    NSString *message=legacy.messages.firstObject;[legacy.messages removeObjectAtIndex:0];
    return strdup(message.UTF8String);
}
void murmur_legacy_remove(const char *script) {
    [legacy.view evaluateJavaScript:[NSString stringWithUTF8String:script] completionHandler:^(id value,NSError *error){(void)value;if(error)legacy.failed=YES;}];
}
void murmur_legacy_stop(void) {
    [legacy.view setNavigationDelegate:nil];[legacy.view stopLoading];
    [(id<MurmurLegacyAPI>)legacy.configuration.userContentController removeScriptMessageHandlerForName:@"migration"];
    legacy.view=nil;legacy.configuration=nil;legacy=nil;
}
