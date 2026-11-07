// Playback only: AVAudioEngine output, never an input node, capture or TCC.
#import <AVFoundation/AVFoundation.h>
#include <stdatomic.h>
// Retain the macOS 14.2 APIs while supporting the newer SDK's checked methods.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
@interface MurmurMeetingAudioEpoch : NSObject {
@public atomic_uint_fast64_t queued; atomic_uint_fast64_t played;
}
@end
@implementation MurmurMeetingAudioEpoch
- (id)init {if((self=[super init])){atomic_init(&queued,0);atomic_init(&played,0);}return self;}
@end
@interface MurmurMeetingOutput : NSObject {
@public AVAudioEngine *engine; AVAudioPlayerNode *player; AVAudioFormat *format;
    uint64_t generation; MurmurMeetingAudioEpoch *epoch;
}
@end
@implementation MurmurMeetingOutput
@end
void *murmur_meeting_output_new(void) { @autoreleasepool {
    MurmurMeetingOutput *o=[MurmurMeetingOutput new];
    o->engine=[AVAudioEngine new];o->player=[AVAudioPlayerNode new];
    o->format=[[AVAudioFormat alloc] initStandardFormatWithSampleRate:16000 channels:1];
    o->epoch=[MurmurMeetingAudioEpoch new];o->generation=0;
    @try {
        [o->engine attachNode:o->player];
        if(@available(macOS 27.0,*)) {
            NSError *error=nil;
            if(![o->engine connect:o->player to:o->engine.mainMixerNode format:o->format error:&error])return NULL;
        }else{[o->engine connect:o->player to:o->engine.mainMixerNode format:o->format];}
    } @catch(NSException *exception) {(void)exception;return NULL;}
    return (__bridge_retained void *)o;
}}
void murmur_meeting_output_reset(void *ptr,uint64_t generation) { @autoreleasepool {
    MurmurMeetingOutput *o=(__bridge MurmurMeetingOutput *)ptr;
    @synchronized(o) {
        if(generation<o->generation)return;
        o->generation=generation;
        [o->player stop];[o->engine stop];
        // Completion callbacks retain their own epoch. Stop cannot deadlock on
        // the output lock and a late completion cannot subtract from a new queue.
        o->epoch=[MurmurMeetingAudioEpoch new];
    }
}}
bool murmur_meeting_output_start(void *ptr,uint64_t generation) { @autoreleasepool {
    MurmurMeetingOutput *o=(__bridge MurmurMeetingOutput *)ptr;
    @synchronized(o) {
        if(generation!=o->generation)return false;
        @try {
            NSError *error=nil;if(![o->engine startAndReturnError:&error])return false;
            if(@available(macOS 27.0,*)){if(![o->player playAndReturnError:&error]){[o->engine stop];return false;}}
            else{[o->player play];}
            return true;
        } @catch(NSException *exception){(void)exception;[o->engine stop];return false;}
    }
}}
bool murmur_meeting_output_push(void *ptr,uint64_t generation,const float *samples,size_t count) { @autoreleasepool {
    MurmurMeetingOutput *o=(__bridge MurmurMeetingOutput *)ptr;
    if(!count||count>1600)return false;
    @synchronized(o) {
        if(generation!=o->generation||!o->engine.running||atomic_load(&o->epoch->queued)+count>4800)return false;
        AVAudioPCMBuffer *buffer=[[AVAudioPCMBuffer alloc] initWithPCMFormat:o->format frameCapacity:(AVAudioFrameCount)count];
        if(!buffer)return false;
        buffer.frameLength=(AVAudioFrameCount)count;memcpy(buffer.floatChannelData[0],samples,count*sizeof(float));
        MurmurMeetingAudioEpoch *epoch=o->epoch;atomic_fetch_add(&epoch->queued,count);
        @try {
            [o->player scheduleBuffer:buffer completionCallbackType:AVAudioPlayerNodeCompletionDataPlayedBack completionHandler:^(AVAudioPlayerNodeCompletionCallbackType type) {
                (void)type;atomic_fetch_add(&epoch->played,count);atomic_fetch_sub(&epoch->queued,count);
            }];return true;
        } @catch(NSException *exception){(void)exception;atomic_fetch_sub(&epoch->queued,count);return false;}
    }
}}
uint64_t murmur_meeting_output_queued(void *ptr) {MurmurMeetingOutput *o=(__bridge MurmurMeetingOutput *)ptr;@synchronized(o){return atomic_load(&o->epoch->queued);}}
uint64_t murmur_meeting_output_played(void *ptr) {MurmurMeetingOutput *o=(__bridge MurmurMeetingOutput *)ptr;@synchronized(o){return atomic_load(&o->epoch->played);}}
void murmur_meeting_output_free(void *ptr) { @autoreleasepool {
    MurmurMeetingOutput *o=CFBridgingRelease(ptr);
    @synchronized(o) {o->generation++;[o->player stop];[o->engine stop];[o->engine detachNode:o->player];}
}}
#pragma clang diagnostic pop
