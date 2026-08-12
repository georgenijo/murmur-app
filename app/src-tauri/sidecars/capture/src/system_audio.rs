//! CATap-backed system-audio capture for meeting sessions.
//!
//! A tap exists only between `SystemAudioStream::start` and `Drop`. The worker
//! never creates one for passive permission polling. Destruction order is the
//! inverse of creation: stop IO, destroy the IO proc, destroy the private
//! aggregate device, then destroy the process tap.

use crate::production::SpscRing;
use murmur_capture_helper_protocol::{CaptureSetupStep, FailureCode, SetupTransition};
use objc2::AnyThread;
use objc2_core_audio::{
    kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceIsStackedKey,
    kAudioAggregateDeviceNameKey, kAudioAggregateDeviceTapAutoStartKey,
    kAudioAggregateDeviceTapListKey, kAudioAggregateDeviceUIDKey, kAudioDevicePermissionsError,
    kAudioDevicePropertyNominalSampleRate, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioSubTapDriftCompensationKey, kAudioSubTapUIDKey,
    AudioDeviceCreateIOProcIDWithBlock, AudioDeviceDestroyIOProcID, AudioDeviceIOProcID,
    AudioDeviceStart, AudioDeviceStop, AudioHardwareCreateAggregateDevice,
    AudioHardwareCreateProcessTap, AudioHardwareDestroyAggregateDevice,
    AudioHardwareDestroyProcessTap, AudioObjectGetPropertyData, AudioObjectID,
    AudioObjectPropertyAddress, CATapDescription, CATapMuteBehavior,
};
use objc2_core_audio_types::{AudioBufferList, AudioTimeStamp};
use objc2_core_foundation::{
    kCFAllocatorDefault, kCFTypeArrayCallBacks, kCFTypeDictionaryKeyCallBacks,
    kCFTypeDictionaryValueCallBacks, CFArray, CFDictionary, CFMutableDictionary, CFRetained,
    CFString,
};
use objc2_foundation::{NSArray, NSNumber, NSString};
use std::ffi::{c_void, CStr};
use std::ptr::NonNull;
use std::sync::Arc;

const MINIMUM_MACOS_MAJOR: u32 = 14;
const MINIMUM_MACOS_MINOR: u32 = 2;

fn os_version() -> Option<(u32, u32)> {
    let name = b"kern.osproductversion\0";
    let mut length = 0_usize;
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr().cast(),
            std::ptr::null_mut(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 || length == 0 || length > 128 {
        return None;
    }
    let mut bytes = vec![0_u8; length];
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr().cast(),
            bytes.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 {
        return None;
    }
    let value = std::str::from_utf8(bytes.get(..length)?)
        .ok()?
        .trim_end_matches('\0');
    let mut parts = value.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

pub(super) fn supported() -> bool {
    os_version().is_some_and(|(major, minor)| {
        major > MINIMUM_MACOS_MAJOR
            || (major == MINIMUM_MACOS_MAJOR && minor >= MINIMUM_MACOS_MINOR)
    })
}

fn to_cfstring(value: &'static CStr) -> CFRetained<CFString> {
    unsafe { CFString::with_c_string(None, value.as_ptr(), 0x0800_0100) }
        .expect("Core Audio dictionary key is a valid CFString")
}

fn aggregate_properties(tap_uid: &NSString, aggregate_uid: &str) -> CFRetained<CFDictionary> {
    let tap_entry = unsafe {
        let dictionary = CFMutableDictionary::new(
            kCFAllocatorDefault,
            2,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        )
        .expect("tap dictionary allocation succeeds");
        CFMutableDictionary::set_value(
            Some(dictionary.as_ref()),
            &*to_cfstring(kAudioSubTapUIDKey) as *const _ as *const c_void,
            tap_uid as *const _ as *const c_void,
        );
        CFMutableDictionary::set_value(
            Some(dictionary.as_ref()),
            &*to_cfstring(kAudioSubTapDriftCompensationKey) as *const _ as *const c_void,
            &*NSNumber::initWithBool(NSNumber::alloc(), false) as *const _ as *const c_void,
        );
        dictionary
    };
    let entries = [tap_entry];
    let taps = unsafe {
        CFArray::new(
            kCFAllocatorDefault,
            entries.as_ptr() as *mut *const c_void,
            entries.len() as isize,
            &kCFTypeArrayCallBacks,
        )
        .expect("tap array allocation succeeds")
    };
    unsafe {
        let dictionary = CFMutableDictionary::new(
            kCFAllocatorDefault,
            6,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        )
        .expect("aggregate dictionary allocation succeeds");
        let name = CFString::from_str("Murmur meeting capture");
        let uid = CFString::from_str(aggregate_uid);
        let enabled = NSNumber::initWithBool(NSNumber::alloc(), true);
        let disabled = NSNumber::initWithBool(NSNumber::alloc(), false);
        for (key, value) in [
            (
                kAudioAggregateDeviceNameKey,
                &*name as *const _ as *const c_void,
            ),
            (
                kAudioAggregateDeviceUIDKey,
                &*uid as *const _ as *const c_void,
            ),
            (
                kAudioAggregateDeviceTapListKey,
                &*taps as *const _ as *const c_void,
            ),
            (
                kAudioAggregateDeviceTapAutoStartKey,
                &*enabled as *const _ as *const c_void,
            ),
            (
                kAudioAggregateDeviceIsPrivateKey,
                &*enabled as *const _ as *const c_void,
            ),
            (
                kAudioAggregateDeviceIsStackedKey,
                &*disabled as *const _ as *const c_void,
            ),
        ] {
            CFMutableDictionary::set_value(
                Some(dictionary.as_ref()),
                &*to_cfstring(key) as *const _ as *const c_void,
                value,
            );
        }
        CFRetained::cast_unchecked::<CFDictionary>(dictionary)
    }
}

fn capture_input_data(ring: &SpscRing, input_data: NonNull<AudioBufferList>) {
    let list = unsafe { input_data.as_ref() };
    let buffers =
        unsafe { std::slice::from_raw_parts(list.mBuffers.as_ptr(), list.mNumberBuffers as usize) };
    // The stable CATap path is stereo. Downmix every returned channel without
    // allocating, locking, or logging on this callback.
    let mut frame_count = usize::MAX;
    let mut channel_count = 0_usize;
    for buffer in buffers {
        let channels = buffer.mNumberChannels as usize;
        if buffer.mData.is_null() || buffer.mDataByteSize == 0 || channels == 0 {
            continue;
        }
        let samples = buffer.mDataByteSize as usize / std::mem::size_of::<f32>();
        frame_count = frame_count.min(samples / channels);
        channel_count += channels;
    }
    if frame_count == usize::MAX || frame_count == 0 || channel_count == 0 {
        return;
    }
    for frame in 0..frame_count {
        let mut sum = 0_f32;
        for buffer in buffers {
            let channels = buffer.mNumberChannels as usize;
            if buffer.mData.is_null() || buffer.mDataByteSize == 0 || channels == 0 {
                continue;
            }
            let samples = unsafe {
                std::slice::from_raw_parts(
                    buffer.mData.cast::<f32>(),
                    buffer.mDataByteSize as usize / std::mem::size_of::<f32>(),
                )
            };
            for channel in 0..channels {
                sum += samples[frame * channels + channel];
            }
        }
        ring.push(sum / channel_count as f32);
    }
}

fn failure_for_status(status: i32) -> FailureCode {
    if status == kAudioDevicePermissionsError {
        FailureCode::PermissionDenied
    } else {
        FailureCode::SystemAudioUnavailable
    }
}

pub(super) struct SystemAudioStream {
    tap_id: AudioObjectID,
    aggregate_id: AudioObjectID,
    io_proc_id: AudioDeviceIOProcID,
    started: bool,
    sample_rate: u32,
}

impl SystemAudioStream {
    pub(super) fn start_observed(
        ring: Arc<SpscRing>,
        mut observe: impl FnMut(CaptureSetupStep, SetupTransition),
    ) -> Result<Self, FailureCode> {
        if !supported() {
            return Err(FailureCode::UnsupportedOs);
        }

        let excluded = NSArray::<NSNumber>::new();
        let description = unsafe {
            CATapDescription::initStereoGlobalTapButExcludeProcesses(
                CATapDescription::alloc(),
                &excluded,
            )
        };
        unsafe {
            description.setName(&NSString::from_str("Murmur meeting system audio"));
            description.setPrivate(true);
            description.setMuteBehavior(CATapMuteBehavior::Unmuted);
        }

        let mut tap_id = 0;
        observe(CaptureSetupStep::SystemTapCreate, SetupTransition::Entered);
        let status =
            unsafe { AudioHardwareCreateProcessTap(Some(description.as_ref()), &mut tap_id) };
        if status != 0 {
            return Err(failure_for_status(status));
        }
        observe(
            CaptureSetupStep::SystemTapCreate,
            SetupTransition::Completed,
        );

        let tap_uid = unsafe { description.UUID().UUIDString() };
        let aggregate_uid = format!(
            "com.localdictation.meeting.{}.{}",
            std::process::id(),
            tap_id
        );
        let properties = aggregate_properties(tap_uid.as_ref(), &aggregate_uid);
        let mut aggregate_id = 0;
        observe(
            CaptureSetupStep::AggregateDeviceCreate,
            SetupTransition::Entered,
        );
        let status = unsafe {
            AudioHardwareCreateAggregateDevice(
                properties.as_ref(),
                NonNull::from(&mut aggregate_id),
            )
        };
        if status != 0 {
            unsafe {
                let _ = AudioHardwareDestroyProcessTap(tap_id);
            }
            return Err(failure_for_status(status));
        }
        observe(
            CaptureSetupStep::AggregateDeviceCreate,
            SetupTransition::Completed,
        );

        let callback_ring = Arc::clone(&ring);
        let io_block = block2::RcBlock::new(
            move |_now: NonNull<AudioTimeStamp>,
                  input_data: NonNull<AudioBufferList>,
                  _input_time: NonNull<AudioTimeStamp>,
                  _output_data: NonNull<AudioBufferList>,
                  _output_time: NonNull<AudioTimeStamp>| {
                capture_input_data(&callback_ring, input_data);
            },
        );
        let mut io_proc_id: AudioDeviceIOProcID = None;
        observe(CaptureSetupStep::IoProcCreate, SetupTransition::Entered);
        let status = unsafe {
            AudioDeviceCreateIOProcIDWithBlock(
                NonNull::from(&mut io_proc_id),
                aggregate_id,
                None,
                block2::RcBlock::as_ptr(&io_block),
            )
        };
        if status != 0 || io_proc_id.is_none() {
            unsafe {
                let _ = AudioHardwareDestroyAggregateDevice(aggregate_id);
                let _ = AudioHardwareDestroyProcessTap(tap_id);
            }
            return Err(failure_for_status(status));
        }
        observe(CaptureSetupStep::IoProcCreate, SetupTransition::Completed);

        let mut sample_rate = 0_f64;
        let mut size = std::mem::size_of::<f64>() as u32;
        let address = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyNominalSampleRate,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };
        let _ = unsafe {
            AudioObjectGetPropertyData(
                aggregate_id,
                NonNull::from(&address),
                0,
                std::ptr::null(),
                NonNull::from(&mut size),
                NonNull::from(&mut sample_rate).cast(),
            )
        };
        let sample_rate = if sample_rate.is_finite() && sample_rate >= 8_000.0 {
            sample_rate.round() as u32
        } else {
            48_000
        };

        observe(CaptureSetupStep::IoProcStart, SetupTransition::Entered);
        let status = unsafe { AudioDeviceStart(aggregate_id, io_proc_id) };
        if status != 0 {
            unsafe {
                let _ = AudioDeviceDestroyIOProcID(aggregate_id, io_proc_id);
                let _ = AudioHardwareDestroyAggregateDevice(aggregate_id);
                let _ = AudioHardwareDestroyProcessTap(tap_id);
            }
            return Err(failure_for_status(status));
        }
        observe(CaptureSetupStep::IoProcStart, SetupTransition::Completed);

        Ok(Self {
            tap_id,
            aggregate_id,
            io_proc_id,
            started: true,
            sample_rate,
        })
    }

    pub(super) fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub(super) fn stop(&mut self) {
        if self.started {
            let stopped = unsafe { AudioDeviceStop(self.aggregate_id, self.io_proc_id) == 0 };
            if stopped {
                self.started = false;
            }
        }
    }
}

impl Drop for SystemAudioStream {
    fn drop(&mut self) {
        self.stop();
        unsafe {
            let _ = AudioDeviceDestroyIOProcID(self.aggregate_id, self.io_proc_id);
            let _ = AudioHardwareDestroyAggregateDevice(self.aggregate_id);
            let _ = AudioHardwareDestroyProcessTap(self.tap_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_version_parser_is_bounded_and_supported_on_test_host() {
        let (major, _) = os_version().expect("macOS product version is available");
        assert!(major >= MINIMUM_MACOS_MAJOR);
        assert_eq!(supported(), major > 14 || os_version().unwrap().1 >= 2);
    }

    #[test]
    fn permission_status_is_typed_without_exposing_osstatus() {
        assert_eq!(
            failure_for_status(kAudioDevicePermissionsError),
            FailureCode::PermissionDenied
        );
        assert_eq!(failure_for_status(-1), FailureCode::SystemAudioUnavailable);
    }
}
