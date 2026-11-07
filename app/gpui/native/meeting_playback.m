// Playback only: AVAudioEngine output, never an input node, capture or TCC.
#import <AVFoundation/AVFoundation.h>
#include <stdatomic.h>
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
@interface MurmurMeetingAudioEpoch : NSObject {
@public atomic_uint_fast64_t queued; atomic_uint_fast64_t played;
    uint64_t generation;
}
@end
@implementation MurmurMeetingAudioEpoch
- (id)init {if((self=[super init])){atomic_init(&queued,0);atomic_init(&played,0);}return self;}
@end
@interface MurmurMeetingOutput : NSObject {
@public AVAudioEngine *engine; AVAudioPlayerNode *player; AVAudioFormat *format;
    dispatch_queue_t worker;
    atomic_uint_fast64_t generation; atomic_uint_fast64_t runningGeneration; atomic_uint_fast64_t configurationEpoch;
    MurmurMeetingAudioEpoch *epoch; id configurationObserver; atomic_bool resetPending;
}
@end
@implementation MurmurMeetingOutput
@end
// The short object lock protects the ARC epoch pointer and reset publication /
// queue insertion. No engine call, dispatch_sync, or notification removal is
// ever made while holding it.
static MurmurMeetingAudioEpoch *current_epoch(MurmurMeetingOutput *o) {
    @synchronized(o) { return o->epoch; }
}
static void stop_output(MurmurMeetingOutput *o) {
    @try {[o->player stop];[o->engine stop];} @catch(NSException *e){(void)e;}
}
void *murmur_meeting_output_new(void) { @autoreleasepool {
    MurmurMeetingOutput *o=[MurmurMeetingOutput new];
    o->worker=dispatch_queue_create("com.murmur.meeting-output",DISPATCH_QUEUE_SERIAL);
    atomic_init(&o->generation,0);atomic_init(&o->runningGeneration,0);atomic_init(&o->configurationEpoch,0);atomic_init(&o->resetPending,false);
    o->epoch=[MurmurMeetingAudioEpoch new];
    return (__bridge_retained void *)o;
}}
void murmur_meeting_output_reset(void *ptr,uint64_t generation) {
    MurmurMeetingOutput *o=(__bridge MurmurMeetingOutput *)ptr;
    // Serialize only publication and queue insertion, so a preempted older
    // reset cannot enqueue a late stop after a newer start has succeeded.
    @synchronized(o) {
    uint_fast64_t prior=atomic_load(&o->generation);
    do {if(generation<=prior)return;}
    while(!atomic_compare_exchange_weak(&o->generation,&prior,generation));
    // Fence first, without waiting on a stalled Core Audio operation.
    // Keep at most one queued reset while start/stop is stalled. The block
    // reads the latest generation rather than retaining one request per click.
    if(atomic_exchange(&o->resetPending,true))return;
    dispatch_async(o->worker, ^{ @autoreleasepool {
        for(;;) {
            uint_fast64_t latest=atomic_load(&o->generation);
            atomic_store(&o->runningGeneration,0);
            stop_output(o);
            MurmurMeetingAudioEpoch *epoch=[MurmurMeetingAudioEpoch new];epoch->generation=latest;
            @synchronized(o) {o->epoch=epoch;}
            atomic_store(&o->resetPending,false);
            // Close the race with cancellation during teardown. A racing reset
            // either queues the next block or lets this one handle its state.
            if(latest==atomic_load(&o->generation)||atomic_exchange(&o->resetPending,true))break;
        }
    }});
    }
}
bool murmur_meeting_output_start(void *ptr,uint64_t generation) {
    if([NSThread isMainThread])return false;
    MurmurMeetingOutput *o=(__bridge MurmurMeetingOutput *)ptr;
    __block bool started=false;
    dispatch_sync(o->worker, ^{ @autoreleasepool {
        if(generation!=atomic_load(&o->generation))return;
        @try {
            if(!o->engine) {
                o->engine=[AVAudioEngine new];o->player=[AVAudioPlayerNode new];
                o->format=[[AVAudioFormat alloc] initStandardFormatWithSampleRate:16000 channels:1];
                [o->engine attachNode:o->player];
                // These selectors are declared by the macOS 26 SDK used in CI.
                // @available alone cannot guard a selector absent from its headers.
                [o->engine connect:o->player to:o->engine.mainMixerNode format:o->format];
            }
            if(!o->configurationObserver) {
                __weak MurmurMeetingOutput *weak=o;
                o->configurationObserver=[[NSNotificationCenter defaultCenter]
                    addObserverForName:AVAudioEngineConfigurationChangeNotification object:o->engine queue:nil
                    usingBlock:^(NSNotification *note) {
                        (void)note;MurmurMeetingOutput *output=weak;if(!output)return;
                        // Invalidate synchronously; teardown alone is queued.
                        atomic_fetch_add(&output->configurationEpoch,1);
                        atomic_store(&output->runningGeneration,0);
                        dispatch_async(output->worker, ^{if(!atomic_load(&output->runningGeneration))stop_output(output);});
                    }];
            }
            // Publish before start so a configuration notification during start
            // can invalidate it; never overwrite that invalidation afterwards.
            uint_fast64_t configuration=atomic_load(&o->configurationEpoch);
            atomic_store(&o->runningGeneration,generation);
            NSError *error=nil;
            if(![o->engine startAndReturnError:&error]){atomic_store(&o->runningGeneration,0);return;}
            if(configuration!=atomic_load(&o->configurationEpoch)||generation!=atomic_load(&o->generation)||generation!=atomic_load(&o->runningGeneration)){atomic_store(&o->runningGeneration,0);stop_output(o);return;}
            [o->player play];
            started=configuration==atomic_load(&o->configurationEpoch)&&generation==atomic_load(&o->generation)&&generation==atomic_load(&o->runningGeneration);
            if(!started){atomic_store(&o->runningGeneration,0);stop_output(o);}
        } @catch(NSException *exception){(void)exception;atomic_store(&o->runningGeneration,0);stop_output(o);}
    }});
    return started;
}
bool murmur_meeting_output_healthy(void *ptr,uint64_t generation) {
    MurmurMeetingOutput *o=(__bridge MurmurMeetingOutput *)ptr;
    return generation==atomic_load(&o->generation)&&generation==atomic_load(&o->runningGeneration);
}
bool murmur_meeting_output_push(void *ptr,uint64_t generation,const float *samples,size_t count) {
    if([NSThread isMainThread]||!count||count>1600)return false;
    MurmurMeetingOutput *o=(__bridge MurmurMeetingOutput *)ptr;
    __block bool pushed=false;
    dispatch_sync(o->worker, ^{ @autoreleasepool {
        MurmurMeetingAudioEpoch *epoch=current_epoch(o);
        if(!murmur_meeting_output_healthy(ptr,generation)||!o->engine.running||atomic_load(&epoch->queued)+count>4800)return;
        AVAudioPCMBuffer *buffer=[[AVAudioPCMBuffer alloc] initWithPCMFormat:o->format frameCapacity:(AVAudioFrameCount)count];
        if(!buffer)return;
        buffer.frameLength=(AVAudioFrameCount)count;memcpy(buffer.floatChannelData[0],samples,count*sizeof(float));
        atomic_fetch_add(&epoch->queued,count);
        @try {
            [o->player scheduleBuffer:buffer completionCallbackType:AVAudioPlayerNodeCompletionDataPlayedBack completionHandler:^(AVAudioPlayerNodeCompletionCallbackType type) {
                (void)type;atomic_fetch_add(&epoch->played,count);atomic_fetch_sub(&epoch->queued,count);
            }];pushed=murmur_meeting_output_healthy(ptr,generation);
        } @catch(NSException *exception){(void)exception;atomic_fetch_sub(&epoch->queued,count);}
    }});
    return pushed;
}
uint64_t murmur_meeting_output_queued(void *ptr) {
    MurmurMeetingOutput *o=(__bridge MurmurMeetingOutput *)ptr;MurmurMeetingAudioEpoch *epoch=current_epoch(o);
    return epoch->generation==atomic_load(&o->generation)?atomic_load(&epoch->queued):0;
}
uint64_t murmur_meeting_output_played(void *ptr) {
    MurmurMeetingOutput *o=(__bridge MurmurMeetingOutput *)ptr;MurmurMeetingAudioEpoch *epoch=current_epoch(o);
    return epoch->generation==atomic_load(&o->generation)?atomic_load(&epoch->played):0;
}
void murmur_meeting_output_free(void *ptr) {
    MurmurMeetingOutput *o=CFBridgingRelease(ptr);
    atomic_fetch_add(&o->generation,1);
    dispatch_async(o->worker, ^{ @autoreleasepool {
        if(o->configurationObserver)[[NSNotificationCenter defaultCenter] removeObserver:o->configurationObserver];
        stop_output(o);
        @try {if(o->player)[o->engine detachNode:o->player];} @catch(NSException *e){(void)e;}
    }});
}
#pragma clang diagnostic pop
