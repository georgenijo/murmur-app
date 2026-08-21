use core_foundation_sys::base::CFTypeRef;
use core_foundation_sys::string::{kCFStringEncodingUTF8, CFStringGetCString, CFStringRef};
use coreaudio::audio_unit::audio_format::LinearPcmFlags;
use coreaudio::audio_unit::macos_helpers::{
    get_audio_device_ids_for_scope, get_default_device_id, get_device_name,
};
use coreaudio::audio_unit::render_callback::{self, data};
use coreaudio::audio_unit::{
    AudioUnit, Element, IOType, SampleFormat as AuSampleFormat, Scope, StreamFormat,
};
use coreaudio::sys::{
    kAudioDevicePropertyDeviceUID, kAudioHardwarePropertyDefaultInputDevice,
    kAudioHardwarePropertyDevices, kAudioObjectPropertyElementMaster,
    kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
    kAudioOutputUnitProperty_CurrentDevice, kAudioOutputUnitProperty_EnableIO, AudioDeviceID,
    AudioObjectAddPropertyListener, AudioObjectGetPropertyData, AudioObjectID,
    AudioObjectPropertyAddress, AudioObjectPropertyListenerProc, AudioObjectRemovePropertyListener,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream};
use murmur_capture_helper_protocol::{
    read_production_frame, write_production_control, write_production_pcm, CaptureBackend,
    CaptureChannel, CapturePhase, CaptureSetupStep, FailureCode, ProductionDevice, ProductionFrame,
    ProductionHelperMessage, ProductionHostMessage, ProductionPcmMetadata, SessionNonce,
    SetupTransition, SystemAudioPermissionStatus, MAX_INPUT_DEVICE_COUNT,
};
use std::ffi::c_void;
use std::io::{Read, Write};
use std::sync::atomic::{
    AtomicBool as ProcessAtomicBool, AtomicU64 as ProcessAtomicU64, Ordering as ProcessOrdering,
};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

#[cfg(test)]
use loom::cell::UnsafeCell;
#[cfg(test)]
use loom::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(not(test))]
use std::cell::UnsafeCell;
#[cfg(not(test))]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const RING_CAPACITY: usize = 48_000 * 8;
const STOP_DEADLINE: Duration = Duration::from_secs(2);
// The passive watcher is the worker's only production session. Keep callback
// state alive until process exit because Core Audio does not promise that a
// failed/best-effort listener removal has drained every racing callback.
static INPUT_TOPOLOGY_CHANGED: ProcessAtomicBool = ProcessAtomicBool::new(false);

pub(super) struct SpscRing {
    slots: Box<[UnsafeCell<f32>]>,
    head: AtomicUsize,
    tail: AtomicUsize,
    overflowed: AtomicBool,
}

/// The latest Core Audio callback clock observed for one channel.
///
/// Only the private Stage 0 AEC feasibility tool constructs this. It records
/// callback anchors alongside drained samples so drift can be measured without
/// changing the production capture protocol.
#[cfg_attr(not(feature = "aec-spike"), allow(dead_code))]
pub(super) struct CallbackClock {
    host_time: ProcessAtomicU64,
    sample_time_bits: ProcessAtomicU64,
}

#[cfg_attr(not(feature = "aec-spike"), allow(dead_code))]
impl CallbackClock {
    pub(super) fn new() -> Self {
        Self {
            host_time: ProcessAtomicU64::new(0),
            sample_time_bits: ProcessAtomicU64::new(0),
        }
    }

    pub(super) fn note(&self, host_time: u64, sample_time: f64) {
        self.sample_time_bits
            .store(sample_time.to_bits(), ProcessOrdering::Relaxed);
        self.host_time.store(host_time, ProcessOrdering::Release);
    }

    pub(super) fn snapshot(&self) -> (u64, f64) {
        let host_time = self.host_time.load(ProcessOrdering::Acquire);
        let sample_time = f64::from_bits(self.sample_time_bits.load(ProcessOrdering::Relaxed));
        (host_time, sample_time)
    }
}

unsafe impl Send for SpscRing {}
unsafe impl Sync for SpscRing {}

impl SpscRing {
    pub(super) fn new() -> Self {
        Self::with_capacity(RING_CAPACITY)
    }

    fn with_capacity(capacity: usize) -> Self {
        assert!(capacity >= 2);
        let mut slots = Vec::with_capacity(capacity);
        slots.resize_with(capacity, || UnsafeCell::new(0.0));
        Self {
            slots: slots.into_boxed_slice(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            overflowed: AtomicBool::new(false),
        }
    }

    pub(super) fn push(&self, value: f32) {
        let head = self.head.load(Ordering::Relaxed);
        let next = (head + 1) % self.slots.len();
        if next == self.tail.load(Ordering::Acquire) {
            self.overflowed.store(true, Ordering::Release);
            return;
        }
        #[cfg(not(test))]
        {
            unsafe { *self.slots[head].get() = value };
        }
        #[cfg(test)]
        {
            self.slots[head].with_mut(|slot| unsafe { *slot = value });
        }
        self.head.store(next, Ordering::Release);
    }

    pub(super) fn drain(&self, output: &mut [f32]) -> usize {
        let mut count = 0;
        let mut tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        while tail != head && count < output.len() {
            #[cfg(not(test))]
            {
                output[count] = unsafe { *self.slots[tail].get() };
            }
            #[cfg(test)]
            {
                self.slots[tail].with(|slot| {
                    output[count] = unsafe { *slot };
                });
            }
            count += 1;
            tail = (tail + 1) % self.slots.len();
        }
        self.tail.store(tail, Ordering::Release);
        count
    }

    pub(super) fn producer_position(&self) -> usize {
        self.head.load(Ordering::Acquire)
    }

    #[cfg_attr(not(feature = "aec-spike"), allow(dead_code))]
    pub(super) fn overflowed(&self) -> bool {
        self.overflowed.load(Ordering::Acquire)
    }
}

pub(super) enum CaptureStream {
    Cpal(Stream),
    Auhal(coreaudio::audio_unit::AudioUnit),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct InputResolutionEvidence {
    input_enumeration_ok: bool,
    requested_present: Option<bool>,
    input_device_count: u16,
    input_device_count_capped: bool,
    default_input_available: bool,
}

impl InputResolutionEvidence {
    fn observed(
        requested_present: Option<bool>,
        input_device_count: usize,
        default_input_available: bool,
    ) -> Self {
        Self {
            input_enumeration_ok: true,
            requested_present,
            input_device_count: input_device_count.min(MAX_INPUT_DEVICE_COUNT) as u16,
            input_device_count_capped: input_device_count > MAX_INPUT_DEVICE_COUNT,
            default_input_available,
        }
    }

    fn enumeration_failed(default_input_available: bool) -> Self {
        Self {
            input_enumeration_ok: false,
            requested_present: None,
            input_device_count: 0,
            input_device_count_capped: false,
            default_input_available,
        }
    }
}

pub(super) enum MicrophoneSetupObservation {
    Step(CaptureSetupStep, SetupTransition),
    InputResolution(InputResolutionEvidence),
}

impl CaptureStream {
    pub(super) fn stop(&mut self) {
        match self {
            Self::Cpal(stream) => {
                let _ = stream.pause();
            }
            Self::Auhal(unit) => {
                let _ = unit.stop();
            }
        }
    }
}

fn parse_nonce(value: &str) -> Result<SessionNonce, ()> {
    if value.len() != 32 {
        return Err(());
    }
    let mut nonce = [0_u8; 16];
    for (index, byte) in nonce.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).map_err(|_| ())?;
    }
    Ok(nonce)
}

fn raw_uid(device: AudioDeviceID) -> Option<String> {
    let address = AudioObjectPropertyAddress {
        mSelector: kAudioDevicePropertyDeviceUID,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMaster,
    };
    let mut value: CFStringRef = std::ptr::null();
    let mut size = std::mem::size_of::<CFTypeRef>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            device,
            &address,
            0,
            std::ptr::null(),
            &mut size,
            &mut value as *mut _ as *mut _,
        )
    };
    if status != 0 || value.is_null() {
        return None;
    }
    let mut bytes = [0_i8; 1024];
    let ok = unsafe {
        CFStringGetCString(
            value,
            bytes.as_mut_ptr(),
            bytes.len() as isize,
            kCFStringEncodingUTF8,
        )
    };
    if ok == 0 {
        return None;
    }
    Some(
        unsafe { std::ffi::CStr::from_ptr(bytes.as_ptr()) }
            .to_string_lossy()
            .into_owned(),
    )
}

fn enumerate() -> Result<(Vec<ProductionDevice>, Option<String>), FailureCode> {
    let ids =
        get_audio_device_ids_for_scope(Scope::Input).map_err(|_| FailureCode::EnumerationFailed)?;
    let default_input_id = get_default_device_id(true).and_then(raw_uid);
    let devices = ids
        .into_iter()
        .filter_map(|id| {
            Some(ProductionDevice {
                id: raw_uid(id)?,
                name: get_device_name(id).ok()?,
            })
        })
        .collect();
    Ok((devices, default_input_id))
}

unsafe extern "C" fn input_topology_changed(
    _object_id: AudioObjectID,
    _address_count: u32,
    _addresses: *const AudioObjectPropertyAddress,
    client_data: *mut c_void,
) -> i32 {
    if let Some(changed) = unsafe { (client_data as *const ProcessAtomicBool).as_ref() } {
        // Core Audio callback boundary: one content-free atomic only. The
        // worker's ordinary control loop serializes the notification later.
        changed.store(true, ProcessOrdering::Release);
    }
    0
}

fn input_topology_address(selector: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMaster,
    }
}

fn watch_input_topology(
    stdout: &mut impl Write,
    capture_id: u64,
    nonce: SessionNonce,
) -> Result<(), ()> {
    INPUT_TOPOLOGY_CHANGED.store(false, ProcessOrdering::Release);
    let client_data = (&INPUT_TOPOLOGY_CHANGED as *const ProcessAtomicBool)
        .cast_mut()
        .cast::<c_void>();
    let devices_address = input_topology_address(kAudioHardwarePropertyDevices);
    let default_address = input_topology_address(kAudioHardwarePropertyDefaultInputDevice);
    let listener: AudioObjectPropertyListenerProc = Some(input_topology_changed);

    let devices_status = unsafe {
        AudioObjectAddPropertyListener(
            kAudioObjectSystemObject,
            &devices_address,
            listener,
            client_data,
        )
    };
    if devices_status != 0 {
        return Err(());
    }
    let default_status = unsafe {
        AudioObjectAddPropertyListener(
            kAudioObjectSystemObject,
            &default_address,
            listener,
            client_data,
        )
    };
    if default_status != 0 {
        unsafe {
            AudioObjectRemovePropertyListener(
                kAudioObjectSystemObject,
                &devices_address,
                listener,
                client_data,
            );
        }
        return Err(());
    }

    let watch_result = (|| -> Result<(), ()> {
        write_production_control(
            stdout,
            capture_id,
            nonce,
            &ProductionHelperMessage::InputTopologyWatchReady,
        )
        .map_err(|_| ())?;

        let (host_sender, host_receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("murmur-input-topology-control".to_string())
            .spawn(move || {
                let mut stdin = std::io::stdin().lock();
                while let Ok(message) = read_control(&mut stdin, capture_id, nonce) {
                    if host_sender.send(message).is_err() {
                        return;
                    }
                }
                // Stdin is the ownership channel. The parent watchdog is a
                // second line of defence, but EOF should reap this worker now.
                unsafe { libc::_exit(0) }
            })
            .map_err(|_| ())?;

        loop {
            if INPUT_TOPOLOGY_CHANGED.swap(false, ProcessOrdering::AcqRel) {
                write_production_control(
                    stdout,
                    capture_id,
                    nonce,
                    &ProductionHelperMessage::InputTopologyChanged,
                )
                .map_err(|_| ())?;
            }
            match host_receiver.recv_timeout(Duration::from_millis(25)) {
                Ok(ProductionHostMessage::Stop | ProductionHostMessage::Cancel) => return Ok(()),
                Ok(_) => return Err(()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(()),
            }
        }
    })();

    unsafe {
        AudioObjectRemovePropertyListener(
            kAudioObjectSystemObject,
            &default_address,
            listener,
            client_data,
        );
        AudioObjectRemovePropertyListener(
            kAudioObjectSystemObject,
            &devices_address,
            listener,
            client_data,
        );
    }
    watch_result?;
    write_production_control(
        stdout,
        capture_id,
        nonce,
        &ProductionHelperMessage::Stopped {
            retained_samples: 0,
        },
    )
    .map_err(|_| ())
}

struct CpalSelection<T> {
    exact_match: Option<T>,
    legacy_match: Option<T>,
    legacy_match_count: usize,
}

impl<T> Default for CpalSelection<T> {
    fn default() -> Self {
        Self {
            exact_match: None,
            legacy_match: None,
            legacy_match_count: 0,
        }
    }
}

impl<T> CpalSelection<T> {
    fn observe(&mut self, candidate: T, id_matches: bool, name_matches: bool) {
        if id_matches {
            // Preserve exact-ID precedence and the prior last-match behavior.
            self.exact_match = Some(candidate);
        } else if name_matches {
            self.legacy_match_count = self.legacy_match_count.saturating_add(1);
            self.legacy_match = (self.legacy_match_count == 1).then_some(candidate);
        }
    }

    fn selected(self) -> Option<T> {
        self.exact_match.or_else(|| {
            if self.legacy_match_count == 1 {
                self.legacy_match
            } else {
                None
            }
        })
    }
}

fn select_auhal_device<T>(
    requested: &str,
    candidates: impl IntoIterator<Item = (T, Option<String>)>,
) -> Option<T> {
    candidates
        .into_iter()
        .find_map(|(candidate, uid)| (uid.as_deref() == Some(requested)).then_some(candidate))
}

fn require_input_device<T>(device: Option<T>) -> Result<T, FailureCode> {
    device.ok_or(FailureCode::NoInputDevice)
}

fn cpal_device(
    requested: Option<&str>,
) -> (Result<cpal::Device, FailureCode>, InputResolutionEvidence) {
    let host = cpal::default_host();
    // Keep the fallback backend independent from Core Audio resolution calls:
    // even evidence-only HAL work could otherwise stall before CPAL gets a
    // chance to resolve the requested device. Reuse CPAL's own default result
    // for both the availability fact and system-default selection.
    let default_input_device = host.default_input_device();
    let default_input_available = default_input_device.is_some();
    if let Some(requested) = requested {
        let devices = match host.input_devices() {
            Ok(devices) => devices,
            Err(_) => {
                return (
                    Err(FailureCode::EnumerationFailed),
                    InputResolutionEvidence::enumeration_failed(default_input_available),
                )
            }
        };
        let mut input_device_count = 0_usize;
        let mut selection = CpalSelection::default();
        for device in devices {
            input_device_count = input_device_count.saturating_add(1);
            let id_matches = device
                .id()
                .ok()
                .map(|id| id.id() == requested)
                .unwrap_or(false);
            let name_matches = device
                .description()
                .ok()
                .map(|description| description.name() == requested)
                .unwrap_or(false);
            selection.observe(device, id_matches, name_matches);
        }
        let selected = require_input_device(selection.selected());
        let evidence = InputResolutionEvidence::observed(
            Some(selected.is_ok()),
            input_device_count,
            default_input_available,
        );
        (selected, evidence)
    } else {
        // Preserve CPAL's default-device result as the capture decision. The
        // supplementary enumeration supplies only a bounded content-free
        // count; failure to collect it must not make default capture fail.
        let selected = require_input_device(default_input_device);
        let evidence = match host.input_devices() {
            Ok(devices) => {
                InputResolutionEvidence::observed(None, devices.count(), default_input_available)
            }
            Err(_) => InputResolutionEvidence::enumeration_failed(default_input_available),
        };
        (selected, evidence)
    }
}

fn build_cpal_for<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    ring: Arc<SpscRing>,
    failed: Arc<AtomicBool>,
) -> Result<Stream, FailureCode>
where
    T: SizedSample + Sample + Send + 'static,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config,
            move |input: &[T], _| {
                for frame in input.chunks(channels) {
                    let mono =
                        frame.iter().copied().map(f32::from_sample).sum::<f32>() / channels as f32;
                    ring.push(mono);
                }
            },
            move |_| failed.store(true, Ordering::Release),
            Some(Duration::from_secs(10)),
        )
        .map_err(|_| FailureCode::StreamOpenFailed)
}

fn start_cpal(
    requested: Option<&str>,
    ring: Arc<SpscRing>,
    failed: Arc<AtomicBool>,
    emit: &mut impl FnMut(MicrophoneSetupObservation) -> Result<(), FailureCode>,
) -> Result<(CaptureStream, u32), FailureCode> {
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::DeviceResolution,
        SetupTransition::Entered,
    ))?;
    let (device, evidence) = cpal_device(requested);
    emit(MicrophoneSetupObservation::InputResolution(evidence))?;
    let device = device?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::DeviceResolution,
        SetupTransition::Completed,
    ))?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::DefaultConfig,
        SetupTransition::Entered,
    ))?;
    let supported = device
        .default_input_config()
        .map_err(|_| FailureCode::ConfigurationFailed)?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::DefaultConfig,
        SetupTransition::Completed,
    ))?;
    let rate = supported.sample_rate();
    let channels = supported.channels() as usize;
    let config = supported.config();
    macro_rules! build {
        ($sample:ty) => {
            build_cpal_for::<$sample>(
                &device,
                config,
                channels,
                Arc::clone(&ring),
                Arc::clone(&failed),
            )
        };
    }
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::StreamBuild,
        SetupTransition::Entered,
    ))?;
    let stream = match supported.sample_format() {
        SampleFormat::I8 => build!(i8),
        SampleFormat::I16 => build!(i16),
        SampleFormat::I24 => build!(cpal::I24),
        SampleFormat::I32 => build!(i32),
        SampleFormat::I64 => build!(i64),
        SampleFormat::U8 => build!(u8),
        SampleFormat::U16 => build!(u16),
        SampleFormat::U24 => build!(cpal::U24),
        SampleFormat::U32 => build!(u32),
        SampleFormat::U64 => build!(u64),
        SampleFormat::F32 => build!(f32),
        SampleFormat::F64 => build!(f64),
        _ => return Err(FailureCode::ConfigurationFailed),
    }?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::StreamBuild,
        SetupTransition::Completed,
    ))?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::StreamStart,
        SetupTransition::Entered,
    ))?;
    stream.play().map_err(|_| FailureCode::StreamStartFailed)?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::StreamStart,
        SetupTransition::Completed,
    ))?;
    Ok((CaptureStream::Cpal(stream), rate))
}

pub(super) fn start_auhal(
    requested: Option<&str>,
    ring: Arc<SpscRing>,
    callback_clock: Option<Arc<CallbackClock>>,
    emit: &mut impl FnMut(MicrophoneSetupObservation) -> Result<(), FailureCode>,
) -> Result<(CaptureStream, u32), FailureCode> {
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::DeviceResolution,
        SetupTransition::Entered,
    ))?;
    let default_device = get_default_device_id(true);
    let default_input_available = default_device.is_some();
    let (device, evidence) = match requested {
        Some(uid) => match get_audio_device_ids_for_scope(Scope::Input) {
            Ok(devices) => {
                let input_device_count = devices.len();
                let selected = require_input_device(select_auhal_device(
                    uid,
                    devices
                        .iter()
                        .copied()
                        .map(|device| (device, raw_uid(device))),
                ));
                let evidence = InputResolutionEvidence::observed(
                    Some(selected.is_ok()),
                    input_device_count,
                    default_input_available,
                );
                (selected, evidence)
            }
            Err(_) => (
                Err(FailureCode::EnumerationFailed),
                InputResolutionEvidence::enumeration_failed(default_input_available),
            ),
        },
        None => {
            // Keep the existing default-device decision independent of the
            // supplementary count enumeration used only for telemetry.
            let selected = require_input_device(default_device);
            let evidence = match get_audio_device_ids_for_scope(Scope::Input) {
                Ok(devices) => {
                    InputResolutionEvidence::observed(None, devices.len(), default_input_available)
                }
                Err(_) => InputResolutionEvidence::enumeration_failed(default_input_available),
            };
            (selected, evidence)
        }
    };
    emit(MicrophoneSetupObservation::InputResolution(evidence))?;
    let device = device?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::DeviceResolution,
        SetupTransition::Completed,
    ))?;
    let sample_rate = 48_000_u32;
    // Inlined from coreaudio-rs audio_unit_from_device_id so every native
    // Core Audio call gets its own Entered/Completed bracket; a hang is then
    // attributable to one named operation instead of the whole creation span.
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::AudioUnitNew,
        SetupTransition::Entered,
    ))?;
    let mut unit = AudioUnit::new(IOType::HalOutput).map_err(|_| FailureCode::StreamOpenFailed)?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::AudioUnitNew,
        SetupTransition::Completed,
    ))?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::EnableInputIo,
        SetupTransition::Entered,
    ))?;
    unit.set_property(
        kAudioOutputUnitProperty_EnableIO,
        Scope::Input,
        Element::Input,
        Some(&1_u32),
    )
    .map_err(|_| FailureCode::StreamOpenFailed)?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::EnableInputIo,
        SetupTransition::Completed,
    ))?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::DisableOutputIo,
        SetupTransition::Entered,
    ))?;
    unit.set_property(
        kAudioOutputUnitProperty_EnableIO,
        Scope::Output,
        Element::Output,
        Some(&0_u32),
    )
    .map_err(|_| FailureCode::StreamOpenFailed)?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::DisableOutputIo,
        SetupTransition::Completed,
    ))?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::SetCurrentDevice,
        SetupTransition::Entered,
    ))?;
    unit.set_property(
        kAudioOutputUnitProperty_CurrentDevice,
        Scope::Global,
        Element::Output,
        Some(&device),
    )
    .map_err(|_| FailureCode::StreamOpenFailed)?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::SetCurrentDevice,
        SetupTransition::Completed,
    ))?;
    let format = StreamFormat {
        sample_rate: sample_rate as f64,
        sample_format: AuSampleFormat::F32,
        flags: LinearPcmFlags::IS_FLOAT
            | LinearPcmFlags::IS_PACKED
            | LinearPcmFlags::IS_NON_INTERLEAVED,
        channels: 1,
    };
    let asbd = format.to_asbd();
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::FormatConfiguration,
        SetupTransition::Entered,
    ))?;
    unit.set_property(
        coreaudio::sys::kAudioUnitProperty_StreamFormat,
        Scope::Output,
        Element::Input,
        Some(&asbd),
    )
    .map_err(|_| FailureCode::ConfigurationFailed)?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::FormatConfiguration,
        SetupTransition::Completed,
    ))?;
    type Args = render_callback::Args<data::NonInterleaved<f32>>;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::CallbackInstallation,
        SetupTransition::Entered,
    ))?;
    unit.set_input_callback(move |args: Args| {
        if let Some(clock) = &callback_clock {
            clock.note(args.time_stamp.mHostTime, args.time_stamp.mSampleTime);
        }
        if let Some(channel) = args.data.channels().next() {
            for sample in channel.iter().take(args.num_frames) {
                ring.push(*sample);
            }
        }
        Ok(())
    })
    .map_err(|_| FailureCode::StreamOpenFailed)?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::CallbackInstallation,
        SetupTransition::Completed,
    ))?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::StreamStart,
        SetupTransition::Entered,
    ))?;
    unit.start().map_err(|_| FailureCode::StreamStartFailed)?;
    emit(MicrophoneSetupObservation::Step(
        CaptureSetupStep::StreamStart,
        SetupTransition::Completed,
    ))?;
    Ok((CaptureStream::Auhal(unit), sample_rate))
}

fn read_control(
    reader: &mut impl Read,
    capture_id: u64,
    nonce: SessionNonce,
) -> Result<ProductionHostMessage, ()> {
    match read_production_frame(reader, capture_id, nonce).map_err(|_| ())? {
        ProductionFrame::Control(message) => Ok(message),
        ProductionFrame::Pcm(_) => Err(()),
    }
}

fn write_meeting_failure(
    stdout: &mut impl Write,
    capture_id: u64,
    nonce: SessionNonce,
    code: FailureCode,
    channel: Option<CaptureChannel>,
    microphone_samples: u64,
    system_samples: u64,
) -> Result<(), ()> {
    write_production_control(
        stdout,
        capture_id,
        nonce,
        &ProductionHelperMessage::MeetingFailure {
            code,
            channel,
            microphone_samples,
            system_samples,
        },
    )
    .map_err(|_| ())
}

fn probe_system_audio(
    stdout: &mut impl Write,
    capture_id: u64,
    nonce: SessionNonce,
) -> Result<(), ()> {
    let ring = Arc::new(SpscRing::new());
    let started = {
        let mut emit_setup = |step: CaptureSetupStep, transition: SetupTransition| {
            let _ = write_production_control(
                stdout,
                capture_id,
                nonce,
                &ProductionHelperMessage::MeetingSetupStep {
                    channel: CaptureChannel::System,
                    step,
                    transition,
                },
            );
        };
        crate::system_audio::SystemAudioStream::start_observed(Arc::clone(&ring), &mut emit_setup)
    };
    let status = match started {
        Ok(mut stream) => {
            let deadline = Instant::now() + STOP_DEADLINE;
            while ring.producer_position() == 0 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            stream.stop();
            drop(stream);
            if ring.producer_position() == 0 {
                return write_meeting_failure(
                    stdout,
                    capture_id,
                    nonce,
                    FailureCode::CallbackStalled,
                    Some(CaptureChannel::System),
                    0,
                    0,
                );
            }
            SystemAudioPermissionStatus::Granted
        }
        Err(FailureCode::PermissionDenied) => SystemAudioPermissionStatus::Denied,
        Err(FailureCode::UnsupportedOs) => SystemAudioPermissionStatus::Unsupported,
        Err(code) => {
            return write_meeting_failure(
                stdout,
                capture_id,
                nonce,
                code,
                Some(CaptureChannel::System),
                0,
                0,
            )
        }
    };
    write_production_control(
        stdout,
        capture_id,
        nonce,
        &ProductionHelperMessage::SystemAudioPermission { status },
    )
    .map_err(|_| ())
}

fn parent_is_gone(original_parent: u32, current_parent: u32) -> bool {
    current_parent <= 1 || current_parent != original_parent
}

#[cfg(target_os = "macos")]
fn spawn_parent_watchdog() -> Result<(), ()> {
    let original_parent = unsafe { libc::getppid() as u32 };
    std::thread::Builder::new()
        .name("murmur-capture-parent-watchdog".to_string())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(100));
            let current_parent = unsafe { libc::getppid() as u32 };
            if parent_is_gone(original_parent, current_parent) {
                // A Core Audio entry point can block forever, so normal Rust
                // unwinding is not dependable after the host disappears.
                unsafe { libc::_exit(0) }
            }
        })
        .map(|_| ())
        .map_err(|_| ())
}

#[cfg(not(target_os = "macos"))]
fn spawn_parent_watchdog() -> Result<(), ()> {
    Ok(())
}

fn run_meeting(
    stdout: &mut impl Write,
    capture_id: u64,
    nonce: SessionNonce,
    device_id: Option<String>,
    backend: CaptureBackend,
    fault: Option<&str>,
) -> Result<(), ()> {
    if !crate::system_audio::supported() {
        return write_meeting_failure(
            stdout,
            capture_id,
            nonce,
            FailureCode::UnsupportedOs,
            Some(CaptureChannel::System),
            0,
            0,
        );
    }

    let meeting_started_at = Instant::now();
    let system_ring = Arc::new(SpscRing::new());
    write_production_control(
        stdout,
        capture_id,
        nonce,
        &ProductionHelperMessage::MeetingPhase {
            phase: CapturePhase::StreamOpen,
            channel: CaptureChannel::System,
        },
    )
    .map_err(|_| ())?;
    let system_started = {
        let mut emit_system_setup = |step: CaptureSetupStep, transition: SetupTransition| {
            let _ = write_production_control(
                stdout,
                capture_id,
                nonce,
                &ProductionHelperMessage::MeetingSetupStep {
                    channel: CaptureChannel::System,
                    step,
                    transition,
                },
            );
        };
        crate::system_audio::SystemAudioStream::start_observed(
            Arc::clone(&system_ring),
            &mut emit_system_setup,
        )
    };
    let mut system_stream = match system_started {
        Ok(stream) => stream,
        Err(code) => {
            return write_meeting_failure(
                stdout,
                capture_id,
                nonce,
                code,
                Some(CaptureChannel::System),
                0,
                0,
            )
        }
    };
    let microphone_ring = Arc::new(SpscRing::new());
    let microphone_failed = Arc::new(AtomicBool::new(false));
    write_production_control(
        stdout,
        capture_id,
        nonce,
        &ProductionHelperMessage::MeetingPhase {
            phase: CapturePhase::StreamOpen,
            channel: CaptureChannel::Microphone,
        },
    )
    .map_err(|_| ())?;
    let microphone_started = {
        let mut emit_microphone_setup = |observation: MicrophoneSetupObservation| {
            let message = match observation {
                MicrophoneSetupObservation::Step(step, transition) => {
                    ProductionHelperMessage::MeetingSetupStep {
                        channel: CaptureChannel::Microphone,
                        step,
                        transition,
                    }
                }
                MicrophoneSetupObservation::InputResolution(evidence) => {
                    ProductionHelperMessage::InputResolution {
                        backend,
                        input_enumeration_ok: evidence.input_enumeration_ok,
                        requested_present: evidence.requested_present,
                        input_device_count: evidence.input_device_count,
                        input_device_count_capped: evidence.input_device_count_capped,
                        default_input_available: evidence.default_input_available,
                    }
                }
            };
            write_production_control(stdout, capture_id, nonce, &message)
                .map_err(|_| FailureCode::Internal)
        };
        match backend {
            CaptureBackend::Cpal => start_cpal(
                device_id.as_deref(),
                Arc::clone(&microphone_ring),
                Arc::clone(&microphone_failed),
                &mut emit_microphone_setup,
            ),
            CaptureBackend::Auhal => start_auhal(
                device_id.as_deref(),
                Arc::clone(&microphone_ring),
                None,
                &mut emit_microphone_setup,
            ),
        }
    };
    let (mut microphone_stream, microphone_rate) = match microphone_started {
        Ok(value) => value,
        Err(code) => {
            system_stream.stop();
            drop(system_stream);
            return write_meeting_failure(
                stdout,
                capture_id,
                nonce,
                code,
                Some(CaptureChannel::Microphone),
                0,
                0,
            );
        }
    };
    let system_rate = system_stream.sample_rate();
    for channel in [CaptureChannel::System, CaptureChannel::Microphone] {
        write_production_control(
            stdout,
            capture_id,
            nonce,
            &ProductionHelperMessage::MeetingPhase {
                phase: CapturePhase::AwaitingFirstCallback,
                channel,
            },
        )
        .map_err(|_| ())?;
    }

    let (control_tx, control_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut input = std::io::stdin().lock();
        if let Ok(message) = read_control(&mut input, capture_id, nonce) {
            let _ = control_tx.send(message);
        }
    });

    let mut microphone_sequence = 0_u64;
    let mut system_sequence = 0_u64;
    let mut microphone_samples = 0_u64;
    let mut system_samples = 0_u64;
    let mut microphone_scratch = [0_f32; 4096];
    let mut system_scratch = [0_f32; 4096];
    let mut microphone_active = false;
    let mut system_active = false;
    let mut last_microphone_position = microphone_ring.producer_position();
    let mut last_system_position = system_ring.producer_position();
    let mut last_microphone_progress = Instant::now();
    let mut last_system_progress = Instant::now();

    loop {
        if microphone_failed.load(Ordering::Acquire) {
            return write_meeting_failure(
                stdout,
                capture_id,
                nonce,
                FailureCode::StreamError,
                Some(CaptureChannel::Microphone),
                microphone_samples,
                system_samples,
            );
        }
        for (channel, ring) in [
            (CaptureChannel::Microphone, &microphone_ring),
            (CaptureChannel::System, &system_ring),
        ] {
            if ring.overflowed.load(Ordering::Acquire) {
                return write_meeting_failure(
                    stdout,
                    capture_id,
                    nonce,
                    FailureCode::Internal,
                    Some(channel),
                    microphone_samples,
                    system_samples,
                );
            }
        }

        let now = Instant::now();
        let microphone_position = microphone_ring.producer_position();
        if microphone_position != last_microphone_position {
            last_microphone_position = microphone_position;
            last_microphone_progress = now;
        }
        let system_position = system_ring.producer_position();
        if system_position != last_system_position {
            last_system_position = system_position;
            last_system_progress = now;
        }

        for (channel, active, last_progress) in [
            (
                CaptureChannel::Microphone,
                microphone_active,
                last_microphone_progress,
            ),
            (CaptureChannel::System, system_active, last_system_progress),
        ] {
            let deadline = if active {
                Duration::from_secs(2)
            } else {
                STOP_DEADLINE
            };
            if last_progress.elapsed() >= deadline {
                return write_meeting_failure(
                    stdout,
                    capture_id,
                    nonce,
                    FailureCode::CallbackStalled,
                    Some(channel),
                    microphone_samples,
                    system_samples,
                );
            }
        }

        let microphone_count = microphone_ring.drain(&mut microphone_scratch);
        if microphone_count > 0 {
            if !microphone_active {
                microphone_active = true;
                write_production_control(
                    stdout,
                    capture_id,
                    nonce,
                    &ProductionHelperMessage::MeetingPhase {
                        phase: CapturePhase::Active,
                        channel: CaptureChannel::Microphone,
                    },
                )
                .map_err(|_| ())?;
            }
            write_production_pcm(
                stdout,
                capture_id,
                nonce,
                ProductionPcmMetadata {
                    channel: CaptureChannel::Microphone,
                    sequence: microphone_sequence,
                    sample_rate: microphone_rate,
                    captured_at_ns: meeting_started_at
                        .elapsed()
                        .as_nanos()
                        .min(u64::MAX as u128) as u64,
                    sample_offset: microphone_samples,
                },
                &microphone_scratch[..microphone_count],
            )
            .map_err(|_| ())?;
            microphone_sequence += 1;
            microphone_samples += microphone_count as u64;
        }

        let system_count = system_ring.drain(&mut system_scratch);
        if system_count > 0 {
            if !system_active {
                system_active = true;
                write_production_control(
                    stdout,
                    capture_id,
                    nonce,
                    &ProductionHelperMessage::MeetingPhase {
                        phase: CapturePhase::Active,
                        channel: CaptureChannel::System,
                    },
                )
                .map_err(|_| ())?;
            }
            write_production_pcm(
                stdout,
                capture_id,
                nonce,
                ProductionPcmMetadata {
                    channel: CaptureChannel::System,
                    sequence: system_sequence,
                    sample_rate: system_rate,
                    captured_at_ns: meeting_started_at
                        .elapsed()
                        .as_nanos()
                        .min(u64::MAX as u128) as u64,
                    sample_offset: system_samples,
                },
                &system_scratch[..system_count],
            )
            .map_err(|_| ())?;
            system_sequence += 1;
            system_samples += system_count as u64;
        }

        match control_rx.try_recv() {
            Ok(ProductionHostMessage::Stop | ProductionHostMessage::Cancel) => {
                write_production_control(
                    stdout,
                    capture_id,
                    nonce,
                    &ProductionHelperMessage::MeetingPhase {
                        phase: CapturePhase::Stopping,
                        channel: CaptureChannel::Microphone,
                    },
                )
                .map_err(|_| ())?;
                microphone_stream.stop();
                system_stream.stop();
                drop(microphone_stream);
                drop(system_stream);
                write_production_control(
                    stdout,
                    capture_id,
                    nonce,
                    &ProductionHelperMessage::MeetingStopped {
                        microphone_samples,
                        system_samples,
                    },
                )
                .map_err(|_| ())?;
                return Ok(());
            }
            Ok(_) => return Err(()),
            Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            Err(mpsc::TryRecvError::Empty) => {}
        }

        if fault == Some("ring-overflow") {
            system_ring.overflowed.store(true, Ordering::Release);
        }
        std::thread::sleep(Duration::from_millis(4));
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn spsc_ring_push_drain_wrap_and_overflow_are_bounded() {
        loom::model(|| {
            let ring = SpscRing::with_capacity(3);
            ring.push(1.0);
            ring.push(2.0);
            assert_eq!(ring.producer_position(), 2);

            ring.push(3.0);
            assert!(ring.overflowed.load(Ordering::Acquire));

            let mut first = [0.0];
            assert_eq!(ring.drain(&mut first), 1);
            assert_eq!(first, [1.0]);

            ring.push(3.0);
            let mut rest = [0.0; 2];
            assert_eq!(ring.drain(&mut rest), 2);
            assert_eq!(rest, [2.0, 3.0]);
            assert_eq!(ring.producer_position(), 0);
        });
    }

    #[test]
    fn spsc_ring_publishes_samples_across_threads() {
        loom::model(|| {
            let ring = loom::sync::Arc::new(SpscRing::with_capacity(2));
            let producer = loom::sync::Arc::clone(&ring);
            let consumer = loom::sync::Arc::clone(&ring);

            let push = loom::thread::spawn(move || producer.push(42.5));
            let drain = loom::thread::spawn(move || {
                let mut output = [0.0];
                (consumer.drain(&mut output) == 1).then_some(output[0])
            });

            push.join().unwrap();
            let concurrent = drain.join().unwrap();
            let mut after_join = [0.0];
            let remaining = ring.drain(&mut after_join);
            match concurrent {
                Some(value) => {
                    assert_eq!(value, 42.5);
                    assert_eq!(remaining, 0);
                }
                None => {
                    assert_eq!(remaining, 1);
                    assert_eq!(after_join, [42.5]);
                }
            }
        });
    }

    #[test]
    fn spsc_ring_reuses_slots_across_threads() {
        let mut model = loom::model::Builder::new();
        model.preemption_bound = Some(3);
        model.check(|| {
            let ring = loom::sync::Arc::new(SpscRing::with_capacity(2));
            let producer = loom::sync::Arc::clone(&ring);
            let consumer = loom::sync::Arc::clone(&ring);

            let push = loom::thread::spawn(move || {
                for value in [1.0, 2.0, 3.0] {
                    let prior_position = producer.producer_position();
                    while producer.producer_position() == prior_position {
                        producer.push(value);
                        loom::thread::yield_now();
                    }
                }
            });
            let drain = loom::thread::spawn(move || {
                let mut values = Vec::new();
                while values.len() < 3 {
                    let mut output = [0.0];
                    if consumer.drain(&mut output) == 1 {
                        values.push(output[0]);
                    }
                    loom::thread::yield_now();
                }
                values
            });

            push.join().unwrap();
            assert_eq!(drain.join().unwrap(), vec![1.0, 2.0, 3.0]);
        });
    }

    #[test]
    fn independent_channel_rings_do_not_cross_contaminate() {
        let mut model = loom::model::Builder::new();
        model.preemption_bound = Some(2);
        model.check(|| {
            let microphone = loom::sync::Arc::new(SpscRing::with_capacity(4));
            let system = loom::sync::Arc::new(SpscRing::with_capacity(4));
            let microphone_writer = loom::sync::Arc::clone(&microphone);
            let system_writer = loom::sync::Arc::clone(&system);
            let mic = loom::thread::spawn(move || {
                microphone_writer.push(1.0);
                microphone_writer.push(2.0);
            });
            let sys = loom::thread::spawn(move || {
                system_writer.push(10.0);
                system_writer.push(20.0);
            });
            mic.join().unwrap();
            sys.join().unwrap();

            let mut microphone_output = [0.0; 2];
            let mut system_output = [0.0; 2];
            assert_eq!(microphone.drain(&mut microphone_output), 2);
            assert_eq!(system.drain(&mut system_output), 2);
            assert_eq!(microphone_output, [1.0, 2.0]);
            assert_eq!(system_output, [10.0, 20.0]);
        });
    }

    #[test]
    fn parse_nonce_accepts_exact_hex_and_rejects_malformed_values() {
        assert_eq!(
            parse_nonce("00112233445566778899aAbBcCdDeEfF"),
            Ok([
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff,
            ])
        );
        assert_eq!(parse_nonce("0011"), Err(()));
        assert_eq!(parse_nonce("00112233445566778899aabbccddeefg"), Err(()));
    }

    #[test]
    fn parent_watchdog_only_fires_after_reparenting() {
        assert!(!parent_is_gone(42, 42));
        assert!(parent_is_gone(42, 1));
        assert!(parent_is_gone(42, 43));
    }

    #[test]
    fn topology_callback_uses_process_lifetime_atomic_state() {
        INPUT_TOPOLOGY_CHANGED.store(false, ProcessOrdering::Release);
        let client_data = (&INPUT_TOPOLOGY_CHANGED as *const ProcessAtomicBool)
            .cast_mut()
            .cast::<c_void>();
        unsafe {
            input_topology_changed(0, 0, std::ptr::null(), client_data);
        }
        assert!(INPUT_TOPOLOGY_CHANGED.load(ProcessOrdering::Acquire));
    }

    #[test]
    fn backend_resolver_classifications_fail_closed_without_identity_leakage() {
        let select_cpal = |matches: &[(bool, bool)]| {
            let mut selection = CpalSelection::default();
            for (index, (id_matches, name_matches)) in matches.iter().copied().enumerate() {
                selection.observe(index, id_matches, name_matches);
            }
            selection.selected()
        };
        assert_eq!(select_cpal(&[]), None);
        assert_eq!(select_cpal(&[(false, false)]), None);
        assert_eq!(select_cpal(&[(false, true)]), Some(0));
        assert_eq!(select_cpal(&[(false, true), (false, true)]), None);
        assert_eq!(select_cpal(&[(false, true), (true, true)]), Some(1));

        let auhal_ids = [Some("uid-a".to_string()), None, Some("uid-b".to_string())];
        let auhal_candidates = || auhal_ids.iter().cloned().enumerate();
        assert_eq!(select_auhal_device("missing", auhal_candidates()), None);
        assert_eq!(select_auhal_device("uid-b", auhal_candidates()), Some(2));
        assert_eq!(
            select_auhal_device("missing", std::iter::empty::<(usize, Option<String>)>()),
            None
        );

        // Both backend-specific selectors and both default-device paths use
        // this same typed fail-closed classification.
        assert_eq!(
            require_input_device::<u32>(None),
            Err(FailureCode::NoInputDevice)
        );
        assert_eq!(require_input_device(Some(42_u32)), Ok(42));
    }

    #[test]
    fn telemetry_count_cap_never_truncates_a_late_device_match() {
        let late_index = MAX_INPUT_DEVICE_COUNT + 3;
        let mut cpal_matches = vec![(false, false); MAX_INPUT_DEVICE_COUNT + 7];
        cpal_matches[late_index] = (true, false);
        let mut cpal_selection = CpalSelection::default();
        for (index, (id_matches, name_matches)) in cpal_matches.iter().copied().enumerate() {
            cpal_selection.observe(index, id_matches, name_matches);
        }
        assert_eq!(cpal_selection.selected(), Some(late_index));

        let mut auhal_ids = vec![None; MAX_INPUT_DEVICE_COUNT + 7];
        auhal_ids[late_index] = Some("late-stable-uid".to_string());
        assert_eq!(
            select_auhal_device("late-stable-uid", auhal_ids.into_iter().enumerate()),
            Some(late_index)
        );

        let evidence = InputResolutionEvidence::observed(None, cpal_matches.len(), true);
        assert_eq!(
            usize::from(evidence.input_device_count),
            MAX_INPUT_DEVICE_COUNT
        );
        assert!(evidence.input_device_count_capped);
    }

    #[test]
    fn input_resolution_evidence_is_bounded_and_keeps_unknown_distinct_from_absent() {
        let absent = InputResolutionEvidence::observed(Some(false), 3, true);
        assert!(absent.input_enumeration_ok);
        assert_eq!(absent.requested_present, Some(false));
        assert_eq!(absent.input_device_count, 3);
        assert!(!absent.input_device_count_capped);
        assert!(absent.default_input_available);

        let system_default = InputResolutionEvidence::observed(None, 0, false);
        assert!(system_default.input_enumeration_ok);
        assert_eq!(system_default.requested_present, None);
        assert_eq!(system_default.input_device_count, 0);
        assert!(!system_default.default_input_available);

        let capped = InputResolutionEvidence::observed(None, MAX_INPUT_DEVICE_COUNT + 17, true);
        assert_eq!(capped.input_device_count, MAX_INPUT_DEVICE_COUNT as u16);
        assert!(capped.input_device_count_capped);

        let unavailable = InputResolutionEvidence::enumeration_failed(true);
        assert!(!unavailable.input_enumeration_ok);
        assert_eq!(unavailable.requested_present, None);
        assert_eq!(unavailable.input_device_count, 0);
        assert!(!unavailable.input_device_count_capped);
        assert!(unavailable.default_input_available);
    }

    #[test]
    fn read_control_accepts_matching_control_frame() {
        let capture_id = 42;
        let nonce = [7; 16];
        let mut bytes = Vec::new();
        write_production_control(&mut bytes, capture_id, nonce, &ProductionHostMessage::Hello)
            .unwrap();

        assert_eq!(
            read_control(&mut Cursor::new(bytes), capture_id, nonce),
            Ok(ProductionHostMessage::Hello)
        );
    }

    #[test]
    fn read_control_rejects_pcm_and_mismatched_identity() {
        let capture_id = 42;
        let nonce = [7; 16];
        let mut pcm = Vec::new();
        write_production_pcm(
            &mut pcm,
            capture_id,
            nonce,
            ProductionPcmMetadata {
                channel: CaptureChannel::Microphone,
                sequence: 0,
                sample_rate: 48_000,
                captured_at_ns: 0,
                sample_offset: 0,
            },
            &[0.25],
        )
        .unwrap();
        assert_eq!(
            read_control(&mut Cursor::new(pcm), capture_id, nonce),
            Err(())
        );

        let mut control = Vec::new();
        write_production_control(
            &mut control,
            capture_id,
            nonce,
            &ProductionHostMessage::Stop,
        )
        .unwrap();
        assert_eq!(
            read_control(&mut Cursor::new(control), capture_id + 1, nonce),
            Err(())
        );
    }
}

pub fn run(arguments: &[String]) -> Result<(), ()> {
    spawn_parent_watchdog()?;
    let capture_id = arguments
        .first()
        .ok_or(())?
        .parse::<u64>()
        .map_err(|_| ())?;
    let nonce = parse_nonce(arguments.get(1).ok_or(())?)?;
    let fault = arguments
        .windows(2)
        .find(|pair| pair[0] == "--fault")
        .map(|pair| pair[1].as_str());
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    if read_control(&mut stdin, capture_id, nonce)? != ProductionHostMessage::Hello {
        return Err(());
    }
    write_production_control(
        &mut stdout,
        capture_id,
        nonce,
        &ProductionHelperMessage::HelloAck,
    )
    .map_err(|_| ())?;
    match read_control(&mut stdin, capture_id, nonce)? {
        ProductionHostMessage::Enumerate => {
            let (devices, default_input_id) = enumerate().map_err(|_| ())?;
            write_production_control(
                &mut stdout,
                capture_id,
                nonce,
                &ProductionHelperMessage::Devices {
                    devices,
                    default_input_id,
                },
            )
            .map_err(|_| ())?;
            Ok(())
        }
        ProductionHostMessage::WatchInputTopology => {
            drop(stdin);
            watch_input_topology(&mut stdout, capture_id, nonce)
        }
        ProductionHostMessage::ProbeSystemAudio => {
            drop(stdin);
            probe_system_audio(&mut stdout, capture_id, nonce)
        }
        ProductionHostMessage::StartMeeting { device_id, backend } => {
            drop(stdin);
            run_meeting(&mut stdout, capture_id, nonce, device_id, backend, fault)
        }
        ProductionHostMessage::Start { device_id, backend } => {
            drop(stdin);
            write_production_control(
                &mut stdout,
                capture_id,
                nonce,
                &ProductionHelperMessage::Phase {
                    phase: CapturePhase::StreamOpen,
                    backend,
                },
            )
            .map_err(|_| ())?;
            if fault == Some("hang-stream-build") {
                loop {
                    std::thread::park();
                }
            }
            let ring = Arc::new(SpscRing::new());
            let failed = Arc::new(AtomicBool::new(false));
            let started = {
                let mut emit_setup = |observation: MicrophoneSetupObservation| {
                    let message = match observation {
                        MicrophoneSetupObservation::Step(step, transition) => {
                            ProductionHelperMessage::SetupStep {
                                backend,
                                step,
                                transition,
                            }
                        }
                        MicrophoneSetupObservation::InputResolution(evidence) => {
                            ProductionHelperMessage::InputResolution {
                                backend,
                                input_enumeration_ok: evidence.input_enumeration_ok,
                                requested_present: evidence.requested_present,
                                input_device_count: evidence.input_device_count,
                                input_device_count_capped: evidence.input_device_count_capped,
                                default_input_available: evidence.default_input_available,
                            }
                        }
                    };
                    write_production_control(&mut stdout, capture_id, nonce, &message)
                        .map_err(|_| FailureCode::Internal)
                };
                match backend {
                    CaptureBackend::Cpal => start_cpal(
                        device_id.as_deref(),
                        Arc::clone(&ring),
                        Arc::clone(&failed),
                        &mut emit_setup,
                    ),
                    CaptureBackend::Auhal => start_auhal(
                        device_id.as_deref(),
                        Arc::clone(&ring),
                        None,
                        &mut emit_setup,
                    ),
                }
            };
            let (mut stream, sample_rate) = match started {
                Ok(value) => value,
                Err(code) => {
                    write_production_control(
                        &mut stdout,
                        capture_id,
                        nonce,
                        &ProductionHelperMessage::Failure {
                            code,
                            backend,
                            retained_samples: 0,
                        },
                    )
                    .map_err(|_| ())?;
                    return Ok(());
                }
            };
            write_production_control(
                &mut stdout,
                capture_id,
                nonce,
                &ProductionHelperMessage::SetupStep {
                    backend,
                    step: CaptureSetupStep::AwaitingFirstCallback,
                    transition: SetupTransition::Entered,
                },
            )
            .map_err(|_| ())?;
            write_production_control(
                &mut stdout,
                capture_id,
                nonce,
                &ProductionHelperMessage::Phase {
                    phase: CapturePhase::AwaitingFirstCallback,
                    backend,
                },
            )
            .map_err(|_| ())?;
            write_production_control(
                &mut stdout,
                capture_id,
                nonce,
                &ProductionHelperMessage::SetupStep {
                    backend,
                    step: CaptureSetupStep::AwaitingFirstCallback,
                    transition: SetupTransition::Completed,
                },
            )
            .map_err(|_| ())?;
            let (control_tx, control_rx) = mpsc::channel();
            std::thread::spawn(move || {
                let mut input = std::io::stdin().lock();
                if let Ok(message) = read_control(&mut input, capture_id, nonce) {
                    let _ = control_tx.send(message);
                }
            });
            let mut sequence = 0_u64;
            let mut retained = 0_u64;
            let mut scratch = [0_f32; 4096];
            let started_at = Instant::now();
            let mut last_producer_position = ring.producer_position();
            let mut last_callback_progress = Instant::now();
            let mut starvation_injected = false;
            loop {
                if fault == Some("hang-before-first-buffer") && retained == 0 {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
                if failed.load(Ordering::Acquire) || ring.overflowed.load(Ordering::Acquire) {
                    let code = if ring.overflowed.load(Ordering::Acquire) {
                        FailureCode::Internal
                    } else {
                        FailureCode::StreamError
                    };
                    write_production_control(
                        &mut stdout,
                        capture_id,
                        nonce,
                        &ProductionHelperMessage::Failure {
                            code,
                            backend,
                            retained_samples: retained,
                        },
                    )
                    .map_err(|_| ())?;
                    return Ok(());
                }
                if fault == Some("ring-overflow") {
                    ring.overflowed.store(true, Ordering::Release);
                    continue;
                }
                let producer_position = ring.producer_position();
                if producer_position != last_producer_position {
                    last_producer_position = producer_position;
                    last_callback_progress = Instant::now();
                } else if retained > 0 && last_callback_progress.elapsed() >= Duration::from_secs(1)
                {
                    write_production_control(
                        &mut stdout,
                        capture_id,
                        nonce,
                        &ProductionHelperMessage::Failure {
                            code: FailureCode::CallbackStalled,
                            backend,
                            retained_samples: retained,
                        },
                    )
                    .map_err(|_| ())?;
                    return Ok(());
                }
                let count = ring.drain(&mut scratch);
                if count > 0 {
                    if retained == 0 {
                        write_production_control(
                            &mut stdout,
                            capture_id,
                            nonce,
                            &ProductionHelperMessage::Phase {
                                phase: CapturePhase::Active,
                                backend,
                            },
                        )
                        .map_err(|_| ())?;
                    }
                    if fault == Some("sequence-gap") && sequence == 2 {
                        sequence += 1;
                    }
                    if fault == Some("malformed-frame") {
                        stdout.write_all(b"BAD!").map_err(|_| ())?;
                        return Ok(());
                    }
                    write_production_pcm(
                        &mut stdout,
                        if fault == Some("stale-capture") {
                            capture_id + 1
                        } else {
                            capture_id
                        },
                        nonce,
                        ProductionPcmMetadata {
                            channel: CaptureChannel::Microphone,
                            sequence,
                            sample_rate,
                            captured_at_ns: started_at.elapsed().as_nanos().min(u64::MAX as u128)
                                as u64,
                            sample_offset: retained,
                        },
                        &scratch[..count],
                    )
                    .map_err(|_| ())?;
                    retained += count as u64;
                    sequence += 1;
                    if fault == Some("callback-starvation") && !starvation_injected {
                        stream.stop();
                        starvation_injected = true;
                    }
                    if fault == Some("exit-after-first-buffer") {
                        return Ok(());
                    }
                }
                match control_rx.try_recv() {
                    Ok(ProductionHostMessage::Stop | ProductionHostMessage::Cancel) => {
                        stream.stop();
                        write_production_control(
                            &mut stdout,
                            capture_id,
                            nonce,
                            &ProductionHelperMessage::Stopped {
                                retained_samples: retained,
                            },
                        )
                        .map_err(|_| ())?;
                        return Ok(());
                    }
                    Ok(_) => return Err(()),
                    Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                    Err(mpsc::TryRecvError::Empty) => {}
                }
                if retained == 0 && started_at.elapsed() > STOP_DEADLINE {
                    write_production_control(
                        &mut stdout,
                        capture_id,
                        nonce,
                        &ProductionHelperMessage::Failure {
                            code: FailureCode::CallbackStalled,
                            backend,
                            retained_samples: 0,
                        },
                    )
                    .map_err(|_| ())?;
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(4));
            }
        }
        _ => Err(()),
    }
}
