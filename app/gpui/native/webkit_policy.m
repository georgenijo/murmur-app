// QA-only own-process policy. Never inspects an AX client or user content.
// The callback uses bounded C storage, no Objective-C, logging or Rust calls.
#import <Foundation/Foundation.h>
#include <mach-o/dyld.h>
#include <mach-o/loader.h>
#include <dlfcn.h>
#include <execinfo.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define MAX_IMAGES 32
#define MAX_FRAMES 96
#define PATH_SIZE 1024
#define SYMBOL_SIZE 512

typedef struct { char image[PATH_SIZE]; char symbol[SYMBOL_SIZE]; } WebkitFrame;
typedef struct {
    char image[PATH_SIZE];
    WebkitFrame frames[MAX_FRAMES];
    unsigned count;
    bool ax, writing_tools;
} WebkitLoad;
static WebkitLoad loads[MAX_IMAGES];
static unsigned load_count, baseline, linkage, overflow;
static atomic_bool registering;
static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t settled = PTHREAD_COND_INITIALIZER;
static unsigned in_flight;
static uint64_t arrivals;
static bool observation_closed, late_load;

// Never wait for finalization in a dyld callback: the caller may own the loader
// lock. A post-inventory arrival terminates this QA process even if inventory
// has already returned, so successful output cannot mask a later load.
static void callback_enter(void) {
    pthread_mutex_lock(&lock);
    in_flight++;
    arrivals++;
    bool late=observation_closed;
    late_load|=late;
    pthread_mutex_unlock(&lock);
    if (late) _exit(86);
}
static void callback_leave(void) {
    pthread_mutex_lock(&lock);
    in_flight--;
    pthread_cond_broadcast(&settled);
    pthread_mutex_unlock(&lock);
}

static bool system_image(const char *path) {
    return !strncmp(path, "/System/Library/", 16) || !strncmp(path, "/usr/lib/", 9);
}
bool murmur_webkit_family(const char *path) {
    return path && (strstr(path, "/WebKit.framework/") ||
        strstr(path, "/WebKitLegacy.framework/") || strstr(path, "/WebCore.framework/") ||
        strstr(path, "/JavaScriptCore.framework/") || strstr(path, "/WebGPU.framework/"));
}

// Frames are ordered from the loader outwards, excluding our observer itself.
// Require bundle activation and incoming HIServices AX IPC via Mach/runloop dispatch.
// App frames below the verified source-1 dispatch are the NSApplication event loop,
// not the load's initiator. A web-view frame anywhere still rejects the chain.
bool murmur_webkit_ax_chain(const WebkitFrame *frames, unsigned count, bool truncated) {
    if (!frames || !count || truncated) return false;
    for (unsigned i=0; i<count; i++) {
        const char *s=frames[i].symbol;
        if (!frames[i].image[0] || !s[0] || strstr(s,"<redacted>") ||
            !strcmp(s,"???") || !strcmp(s,"<unknown>")) return false;
        if (strstr(s,"WKWebView") || strstr(s,"WebView") || strstr(s,"webkit")) return false;
    }
    // A dispatch marker alone is only a prefix. Require a fully symbolicated
    // unwind through the event loop to the actual process/thread root.
    const WebkitFrame *root=&frames[count-1];
    if (!((!strcmp(root->image,"/usr/lib/dyld") && !strcmp(root->symbol,"start")) ||
          (!strcmp(root->image,"/usr/lib/system/libsystem_pthread.dylib") &&
           (!strcmp(root->symbol,"thread_start") || !strcmp(root->symbol,"_pthread_start"))))) return false;
    bool bundle=false, services=false, ipc=false, mig=false;
    for (unsigned i=0; i<count; i++) {
        const char *p=frames[i].image, *s=frames[i].symbol;
        if (!system_image(p)) return false; // Includes unknown and Murmur origins.
        if (strstr(p,"/AccessibilityBundles.framework/") && strstr(s,"AXBBundleManager")) bundle=true;
        if (strstr(p,"/HIServices.framework/") && !strcmp(s,"loadAccessibilityBundles")) services=true;
        if (strstr(p,"/HIServices.framework/")) {
            if (!strncmp(s,"_AXXMIG",7)) {
                if (!bundle || !services) return false;
                ipc=true;
            }
            if (ipc && !strcmp(s,"mshMIGPerform")) mig=true;
        }
        if (ipc && mig && strstr(p,"/CoreFoundation.framework/") &&
            !strcmp(s,"__CFRUNLOOP_IS_CALLING_OUT_TO_A_SOURCE1_PERFORM_FUNCTION__")) return true;
    }
    return false;
}

// AppKit probes Writing Tools while customizing a bundled app's main menu,
// even without an Edit menu or text input. This is not a web-view construction.
// Accept only the complete, adjacent system call chain through finishLaunching
// and run; named markers in an app frame or a partial unwind are not evidence.
bool murmur_webkit_writing_tools_chain(const WebkitFrame *frames, unsigned count, bool truncated) {
    if (!frames || !count || count>MAX_FRAMES || truncated) return false;
    for (unsigned i=0; i<count; i++) {
        const char *s=frames[i].symbol;
        if (!frames[i].image[0] || !s[0] || strstr(s,"<redacted>") ||
            !strcmp(s,"???") || !strcmp(s,"<unknown>")) return false;
        if (strstr(s,"WKWebView") || strstr(s,"WebView") || strstr(s,"webkit")) return false;
    }
    const WebkitFrame *root=&frames[count-1];
    if (strcmp(root->image,"/usr/lib/dyld") || strcmp(root->symbol,"start")) return false;
    unsigned i=0;
    // dyld may expose its internal frames under the literal dyld-internal
    // symbol. Preserve those names as returned by dladdr; never fill gaps.
    while (i<count && !strcmp(frames[i].image,"/usr/lib/dyld")) i++;
    static const char *symbols[]={
        "_sl_dlopen",
        "WritingToolsUILibraryCore",
        "+[NSWritingToolsCoordinator isWritingToolsAvailable]",
        "-[NSApplication(NSMenuUpdating) _addTextInputMenuItems:]",
        "-[NSApplication(NSMenuUpdating) _customizeMainMenu]",
        "-[NSApplication finishLaunching]",
        "_NSApplicationBeginRunning",
        "-[NSApplication run]",
    };
    unsigned n=sizeof(symbols)/sizeof(*symbols);
    if (i+n>=count) return false;
    for (unsigned j=0; j<n; j++) {
        const WebkitFrame *f=&frames[i+j];
        const char *image=j==0 ?
            "/System/Library/PrivateFrameworks/SoftLinking.framework/Versions/A/SoftLinking" :
            "/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit";
        if (strcmp(f->image,image) || strcmp(f->symbol,symbols[j])) return false;
    }
    return true;
}

// Check every load command of the executable and non-system dylibs, including
// late dlopen dependencies. Apple frameworks may themselves depend on WebKit.
static bool links_webkit(const struct mach_header *header) {
    if (header->magic != MH_MAGIC_64) return true; // Unsupported format fails closed.
    const struct mach_header_64 *h=(const void *)header;
    const char *p=(const char *)(h+1), *end=p+h->sizeofcmds;
    for (uint32_t i=0; i<h->ncmds; i++) {
        if (p+sizeof(struct load_command)>end) return true;
        const struct load_command *c=(const void *)p;
        if (c->cmdsize<sizeof(*c) || c->cmdsize>(size_t)(end-p)) return true;
        if (c->cmd==LC_LOAD_DYLIB || c->cmd==LC_LOAD_WEAK_DYLIB ||
            c->cmd==LC_REEXPORT_DYLIB || c->cmd==LC_LOAD_UPWARD_DYLIB || c->cmd==LC_LAZY_LOAD_DYLIB) {
            if (c->cmdsize<sizeof(struct dylib_command)) return true;
            const struct dylib_command *d=(const void *)p;
            uint32_t offset=d->dylib.name.offset;
            if (offset>=c->cmdsize || !memchr(p+offset,0,c->cmdsize-offset)) return true;
            if (murmur_webkit_family(p+offset)) return true;
        }
        p+=c->cmdsize;
    }
    return false;
}

__attribute__((noinline)) static void image_added(const struct mach_header *header, intptr_t slide) {
    callback_enter(); // Before path lookup, backtrace or symbolication.
#ifdef MURMUR_WEBKIT_CAPTURE_HOOK
    MURMUR_WEBKIT_CAPTURE_HOOK();
#endif
    (void)slide;
    Dl_info info={0};
    const char *path=dladdr(header,&info) ? info.dli_fname : NULL;
    bool bad_link=!path || (!system_image(path) && links_webkit(header));
    bool web=murmur_webkit_family(path);
    if (!web) {
        pthread_mutex_lock(&lock);
        linkage+=bad_link;
        pthread_mutex_unlock(&lock);
        callback_leave();
        return;
    }
    bool initial=atomic_load(&registering);
    WebkitLoad load={0};
    bool truncated=strlcpy(load.image,path,sizeof(load.image))>=sizeof(load.image);
    void *addresses[MAX_FRAMES+1];
    int n=backtrace(addresses,MAX_FRAMES+1);
    truncated|=n==MAX_FRAMES+1;
    // Frame 0 is this observer. It is never evidence of an app-owned load.
    // Do not hold our mutex across unwinding/symbol lookup (possible reentry).
    for (int i=1; i<n; i++) {
        Dl_info f={0};
        dladdr(addresses[i],&f);
        WebkitFrame *frame=&load.frames[load.count++];
        truncated|=strlcpy(frame->image,f.dli_fname ?: "",PATH_SIZE)>=PATH_SIZE;
        truncated|=strlcpy(frame->symbol,f.dli_sname ?: "",SYMBOL_SIZE)>=SYMBOL_SIZE;
    }
    load.ax=!initial && murmur_webkit_ax_chain(load.frames,load.count,truncated);
    load.writing_tools=!initial && !load.ax &&
        murmur_webkit_writing_tools_chain(load.frames,load.count,truncated);
    pthread_mutex_lock(&lock);
    linkage+=bad_link;
    baseline+=initial;
    if (load_count==MAX_IMAGES) overflow++;
    else loads[load_count++]=load;
    pthread_mutex_unlock(&lock);
    callback_leave();
}

// Registered before main, NSApplication, or window/AX exposure. dyld reports
// existing images synchronously: those form the strict zero-image baseline.
__attribute__((constructor)) static void install_policy(void) {
    atomic_store(&registering,true);
    _dyld_register_func_for_add_image(image_added);
    atomic_store(&registering,false);
}

bool murmur_webkit_start(void) {
    pthread_mutex_lock(&lock);
    bool ok=baseline==0 && linkage==0 && overflow==0 && load_count==0;
    unsigned initial=load_count;
    pthread_mutex_unlock(&lock);
    printf("MURMUR_WEBKIT_BASELINE={\"pid\":%d,\"webkitImages\":%u}\n",getpid(),initial);
    fflush(stdout);
    return ok;
}

bool murmur_webkit_inventory(const char *prefix) {
    // Copy under lock; Foundation/JSON work happens after unlocking because
    // it may itself load a framework. Storage is bounded and QA-only.
    WebkitLoad *snapshot=calloc(MAX_IMAGES,sizeof(WebkitLoad));
    if (!snapshot) return false;
    unsigned count=0, extra=0, base=0, linked=0, printed=0, ax=0, writing_tools=0;
    uint64_t observed=0;
    bool stable=false;
    // A valid AX load arriving during JSON emission must be included, not
    // rejected merely for changing the inventory. Each pass emits new records
    // only; bounded storage limits this to at most MAX_IMAGES+1 snapshots.
    for (unsigned pass=0; pass<=MAX_IMAGES; pass++) {
        pthread_mutex_lock(&lock);
        // cond_wait releases our mutex. Never hold it while acquiring dyld's
        // loader lock (including Foundation allocation/serialization/output).
        while (in_flight) {
#ifdef MURMUR_WEBKIT_WAIT_HOOK
            MURMUR_WEBKIT_WAIT_HOOK();
#endif
            pthread_cond_wait(&settled,&lock);
        }
        observed=arrivals;
        count=load_count; extra=overflow; base=baseline; linked=linkage;
        memcpy(snapshot,loads,count*sizeof(WebkitLoad));
        pthread_mutex_unlock(&lock);
#ifdef MURMUR_WEBKIT_SNAPSHOT_HOOK
        // Unit regression injects an arrival at this precise race boundary.
        MURMUR_WEBKIT_SNAPSHOT_HOOK();
#endif
        @autoreleasepool {
            for (unsigned i=printed; i<count; i++) {
                WebkitLoad *load=&snapshot[i];
                ax+=load->ax;
                writing_tools+=load->writing_tools;
                NSMutableArray *chain=[NSMutableArray array];
                for (unsigned j=0; j<load->count; j++) {
                    WebkitFrame *f=&load->frames[j];
                    [chain addObject:@{@"image": @(f->image), @"symbol": @(f->symbol)}];
                }
                NSDictionary *event=@{@"image": @(load->image), @"axAttributed": @(load->ax),
                    @"writingToolsAttributed": @(load->writing_tools),
                    @"attribution": load->writing_tools ? @"AppKit Writing Tools availability probe" :
                        load->ax ? @"Accessibility bundle activation" : @"unattributed", @"chain": chain};
                NSData *json=[NSJSONSerialization dataWithJSONObject:event options:NSJSONWritingSortedKeys error:NULL];
                printf("MURMUR_WEBKIT_LOAD=%s\n",[[[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding] UTF8String]);
            }
        }
        printed=count;
        pthread_mutex_lock(&lock);
        stable=in_flight==0 && observed==arrivals && count==load_count &&
            extra==overflow && base==baseline && linked==linkage;
        // Serialization is complete. Close observation atomically with the
        // last stability check; all later callbacks fail the process closed.
        if (stable) observation_closed=true;
        pthread_mutex_unlock(&lock);
        if (stable) break;
    }
#ifdef MURMUR_WEBKIT_OUTPUT_HOOK
    MURMUR_WEBKIT_OUTPUT_HOOK();
#endif
    free(snapshot);
    printf("%s={\"pid\":%d,\"webkitImages\":%u,\"axAttributed\":%u,\"writingToolsAttributed\":%u,\"unattributed\":%u,\"startupWebkitImages\":%u,\"linkageViolations\":%u}\n",
        prefix,getpid(),count+extra,ax,writing_tools,count+extra-ax-writing_tools,base,linked);
    bool flushed=fflush(stdout)==0;
#ifdef MURMUR_WEBKIT_FLUSH_HOOK
    MURMUR_WEBKIT_FLUSH_HOOK();
#endif
    pthread_mutex_lock(&lock);
    bool ok=stable && !late_load && in_flight==0 && observed==arrivals;
    pthread_mutex_unlock(&lock);
    return flushed && ok && base==0 && linked==0 && extra==0 && count==ax+writing_tools;
}
