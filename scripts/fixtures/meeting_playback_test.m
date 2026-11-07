// Synthetic output doubles only: no hardware audio, windows, capture or user data.
#import <AVFoundation/AVFoundation.h>
#include <assert.h>
#include <stdatomic.h>
#include <time.h>
#import "../../app/gpui/native/meeting_playback.m"
static dispatch_semaphore_t entered, releaseStart;
static atomic_int starts, stops;
@interface SyntheticEngine : NSObject
@end
@implementation SyntheticEngine
- (BOOL)startAndReturnError:(NSError **)error {
    (void)error;assert(!NSThread.isMainThread);
    atomic_fetch_add(&starts,1);
    dispatch_semaphore_signal(entered);
    dispatch_semaphore_wait(releaseStart,DISPATCH_TIME_FOREVER);
    return YES;
}
- (BOOL)isRunning {return YES;}
- (void)stop {assert(!NSThread.isMainThread);atomic_fetch_add(&stops,1);}
- (void)detachNode:(id)node {(void)node;assert(!NSThread.isMainThread);}
@end
@interface SyntheticPlayer : NSObject
@end
@implementation SyntheticPlayer
- (void)play {assert(!NSThread.isMainThread);}
- (void)stop {assert(!NSThread.isMainThread);}
@end
int main(void) { @autoreleasepool {
    entered=dispatch_semaphore_create(0);releaseStart=dispatch_semaphore_create(0);
    void *ptr=murmur_meeting_output_new();
    MurmurMeetingOutput *o=(__bridge MurmurMeetingOutput *)ptr;
    // Inject without creating AVAudioEngine, so tests cannot open an audio device.
    o->engine=(AVAudioEngine *)[SyntheticEngine new];
    o->player=(AVAudioPlayerNode *)[SyntheticPlayer new];
    murmur_meeting_output_reset(ptr,1);
    dispatch_semaphore_t finished=dispatch_semaphore_create(0);
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_DEFAULT,0), ^{
        assert(!murmur_meeting_output_start(ptr,1));
        dispatch_semaphore_signal(finished);
    });
    assert(!dispatch_semaphore_wait(entered,dispatch_time(DISPATCH_TIME_NOW,NSEC_PER_SEC*5)));
    // Simulate a UI pause during a stuck engine start. Reset and counter reads
    // must return before we permit that engine operation to complete.
    int stopsBefore=atomic_load(&stops);
    for(uint64_t g=2;g<=2000;g++)murmur_meeting_output_reset(ptr,g);
    assert(atomic_load(&o->resetPending));
    assert(atomic_load(&starts)==1 && atomic_load(&stops)==stopsBefore);
    assert(murmur_meeting_output_queued(ptr)==0);
    assert(murmur_meeting_output_played(ptr)==0);
    assert(!murmur_meeting_output_healthy(ptr,1));
    dispatch_semaphore_signal(releaseStart);
    assert(!dispatch_semaphore_wait(finished,dispatch_time(DISPATCH_TIME_NOW,NSEC_PER_SEC*5)));
    dispatch_sync(o->worker, ^{});
    assert(!atomic_load(&o->resetPending));
    assert(atomic_load(&stops)==stopsBefore+2); // stale start + one coalesced stop
    assert(current_epoch(o)->generation==2000);
    assert(atomic_load(&o->generation)==2000);
    // An old reset cannot revoke the newer generation.
    murmur_meeting_output_reset(ptr,1);
    assert(atomic_load(&o->generation)==2000);
    dispatch_semaphore_signal(releaseStart);
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_DEFAULT,0), ^{
        assert(murmur_meeting_output_start(ptr,2000));
        dispatch_semaphore_signal(finished);
    });
    assert(!dispatch_semaphore_wait(finished,dispatch_time(DISPATCH_TIME_NOW,NSEC_PER_SEC*5)));
    assert(murmur_meeting_output_healthy(ptr,2000));
    MurmurMeetingAudioEpoch *old=current_epoch(o);
    atomic_store(&old->queued,4800);
    assert(murmur_meeting_output_queued(ptr)==4800);
    [[NSNotificationCenter defaultCenter] postNotificationName:AVAudioEngineConfigurationChangeNotification object:o->engine];
    assert(!murmur_meeting_output_healthy(ptr,2000));
    dispatch_sync(o->worker, ^{});
    murmur_meeting_output_reset(ptr,2001);
    dispatch_sync(o->worker, ^{});
    // Late completion of the old output cannot subtract from a new epoch.
    atomic_fetch_sub(&old->queued,1600);atomic_fetch_add(&old->played,1600);
    assert(murmur_meeting_output_queued(ptr)==0 && murmur_meeting_output_played(ptr)==0);
    dispatch_semaphore_signal(releaseStart);
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_DEFAULT,0), ^{
        assert(murmur_meeting_output_start(ptr,2001));
        dispatch_semaphore_signal(finished);
    });
    assert(!dispatch_semaphore_wait(finished,dispatch_time(DISPATCH_TIME_NOW,NSEC_PER_SEC*5)));
    assert(murmur_meeting_output_healthy(ptr,2001));
    // Destruction also returns while the audio worker is stalled.
    dispatch_async(o->worker, ^{
        dispatch_semaphore_signal(entered);
        dispatch_semaphore_wait(releaseStart,DISPATCH_TIME_FOREVER);
    });
    // Drain the signals from the two recovery starts and the stalled block.
    assert(!dispatch_semaphore_wait(entered,dispatch_time(DISPATCH_TIME_NOW,NSEC_PER_SEC*5)));
    assert(!dispatch_semaphore_wait(entered,dispatch_time(DISPATCH_TIME_NOW,NSEC_PER_SEC*5)));
    assert(!dispatch_semaphore_wait(entered,dispatch_time(DISPATCH_TIME_NOW,NSEC_PER_SEC*5)));
    murmur_meeting_output_free(ptr);
    dispatch_semaphore_signal(releaseStart);
    dispatch_sync(o->worker, ^{});
    assert(atomic_load(&starts)==3);
    assert(atomic_load(&stops)>=3);
}return 0;}
