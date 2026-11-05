#import <AppKit/AppKit.h>
#import <Carbon/Carbon.h>
int main(int argc, const char **argv) {
    @autoreleasepool {
        if (argc != 2) return 2;
        pid_t pid = atoi(argv[1]);
        NSRunningApplication *app = [NSRunningApplication runningApplicationWithProcessIdentifier:pid];
        if (![app.bundleIdentifier isEqualToString:@"com.georgenijo.gpui0e"]) {
            fprintf(stderr,"REFUSE: PID is not the scratch bundle\n"); return 2;
        }
        NSAppleEventDescriptor *target = [NSAppleEventDescriptor descriptorWithDescriptorType:typeKernelProcessID bytes:&pid length:sizeof(pid)];
        NSAppleEventDescriptor *event = [NSAppleEventDescriptor appleEventWithEventClass:kCoreEventClass eventID:kAEQuitApplication targetDescriptor:target returnID:kAutoGenerateReturnID transactionID:kAnyTransactionID];
        [event setParamDescriptor:[NSAppleEventDescriptor descriptorWithEnumCode:kAEReallyLogOut] forKeyword:kAEQuitReason];
        NSError *error = nil;
        fprintf(stderr,"Sending ONLY to scratch pid=%d event=%s\n",pid,event.description.UTF8String);
        [event sendEventWithOptions:NSAppleEventSendNoReply timeout:10 error:&error];
        if (error) { fprintf(stderr,"%s\n",error.description.UTF8String); return 1; }
        return 0;
    }
}
