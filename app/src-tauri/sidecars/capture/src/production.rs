use core_foundation_sys::base::CFTypeRef;
use core_foundation_sys::string::{kCFStringEncodingUTF8, CFStringGetCString, CFStringRef};
use coreaudio::audio_unit::audio_format::LinearPcmFlags;
use coreaudio::audio_unit::macos_helpers::{
    audio_unit_from_device_id, get_audio_device_ids_for_scope, get_default_device_id,
    get_device_name,
};
use coreaudio::audio_unit::render_callback::{self, data};
use coreaudio::audio_unit::{Element, SampleFormat as AuSampleFormat, Scope, StreamFormat};
use coreaudio::sys::{
    kAudioDevicePropertyDeviceUID, kAudioObjectPropertyElementMaster,
    kAudioObjectPropertyScopeGlobal, AudioDeviceID, AudioObjectGetPropertyData,
    AudioObjectPropertyAddress,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream};
use murmur_capture_helper_protocol::{
    read_production_frame, write_production_control, write_production_pcm, CaptureBackend,
    CapturePhase, FailureCode, ProductionDevice, ProductionFrame, ProductionHelperMessage,
    ProductionHostMessage, SessionNonce,
};
use std::cell::UnsafeCell;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const RING_CAPACITY: usize = 48_000 * 8;
const STOP_DEADLINE: Duration = Duration::from_secs(2);

struct SpscRing {
    slots: Box<[UnsafeCell<f32>]>,
    head: AtomicUsize,
    tail: AtomicUsize,
    overflowed: AtomicBool,
}

unsafe impl Send for SpscRing {}
unsafe impl Sync for SpscRing {}

impl SpscRing {
    fn new() -> Self {
        let mut slots = Vec::with_capacity(RING_CAPACITY);
        slots.resize_with(RING_CAPACITY, || UnsafeCell::new(0.0));
        Self {
            slots: slots.into_boxed_slice(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            overflowed: AtomicBool::new(false),
        }
    }

    fn push(&self, value: f32) {
        let head = self.head.load(Ordering::Relaxed);
        let next = (head + 1) % self.slots.len();
        if next == self.tail.load(Ordering::Acquire) {
            self.overflowed.store(true, Ordering::Release);
            return;
        }
        unsafe { *self.slots[head].get() = value };
        self.head.store(next, Ordering::Release);
    }

    fn drain(&self, output: &mut [f32]) -> usize {
        let mut count = 0;
        let mut tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        while tail != head && count < output.len() {
            output[count] = unsafe { *self.slots[tail].get() };
            count += 1;
            tail = (tail + 1) % self.slots.len();
        }
        self.tail.store(tail, Ordering::Release);
        count
    }

    fn producer_position(&self) -> usize {
        self.head.load(Ordering::Acquire)
    }
}

enum CaptureStream {
    Cpal(Stream),
    Auhal(coreaudio::audio_unit::AudioUnit),
}

impl CaptureStream {
    fn stop(&mut self) {
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

fn enumerate() -> Result<Vec<ProductionDevice>, FailureCode> {
    let ids =
        get_audio_device_ids_for_scope(Scope::Input).map_err(|_| FailureCode::EnumerationFailed)?;
    Ok(ids
        .into_iter()
        .filter_map(|id| {
            Some(ProductionDevice {
                id: raw_uid(id)?,
                name: get_device_name(id).ok()?,
            })
        })
        .collect())
}

fn cpal_device(requested: Option<&str>) -> Result<cpal::Device, FailureCode> {
    let host = cpal::default_host();
    if let Some(requested) = requested {
        let devices = host
            .input_devices()
            .map_err(|_| FailureCode::EnumerationFailed)?;
        let mut exact_match = None;
        let mut legacy_matches = Vec::new();
        for device in devices {
            let id_matches = device
                .id()
                .ok()
                .map(|id| id.id().to_string() == requested)
                .unwrap_or(false);
            let name_matches = device
                .description()
                .ok()
                .map(|description| description.name().to_string() == requested)
                .unwrap_or(false);
            if id_matches {
                exact_match = Some(device);
            } else if name_matches {
                legacy_matches.push(device);
            }
        }
        exact_match
            .or_else(|| {
                (legacy_matches.len() == 1)
                    .then(|| legacy_matches.into_iter().next().expect("length checked"))
            })
            .ok_or(FailureCode::NoInputDevice)
    } else {
        host.default_input_device()
            .ok_or(FailureCode::NoInputDevice)
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
) -> Result<(CaptureStream, u32), FailureCode> {
    let device = cpal_device(requested)?;
    let supported = device
        .default_input_config()
        .map_err(|_| FailureCode::ConfigurationFailed)?;
    let rate = supported.sample_rate();
    let channels = supported.channels() as usize;
    let config = supported.config();
    macro_rules! build {
        ($sample:ty) => {
            build_cpal_for::<$sample>(
                &device,
                config.clone(),
                channels,
                Arc::clone(&ring),
                Arc::clone(&failed),
            )
        };
    }
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
    stream.play().map_err(|_| FailureCode::StreamStartFailed)?;
    Ok((CaptureStream::Cpal(stream), rate))
}

fn start_auhal(
    requested: Option<&str>,
    ring: Arc<SpscRing>,
) -> Result<(CaptureStream, u32), FailureCode> {
    let device = match requested {
        Some(uid) => get_audio_device_ids_for_scope(Scope::Input)
            .map_err(|_| FailureCode::EnumerationFailed)?
            .into_iter()
            .find(|id| raw_uid(*id).as_deref() == Some(uid))
            .ok_or(FailureCode::NoInputDevice)?,
        None => get_default_device_id(true).ok_or(FailureCode::NoInputDevice)?,
    };
    let sample_rate = 48_000_u32;
    let mut unit =
        audio_unit_from_device_id(device, true).map_err(|_| FailureCode::StreamOpenFailed)?;
    let format = StreamFormat {
        sample_rate: sample_rate as f64,
        sample_format: AuSampleFormat::F32,
        flags: LinearPcmFlags::IS_FLOAT
            | LinearPcmFlags::IS_PACKED
            | LinearPcmFlags::IS_NON_INTERLEAVED,
        channels: 1,
    };
    let asbd = format.to_asbd();
    unit.set_property(
        coreaudio::sys::kAudioUnitProperty_StreamFormat,
        Scope::Output,
        Element::Input,
        Some(&asbd),
    )
    .map_err(|_| FailureCode::ConfigurationFailed)?;
    type Args = render_callback::Args<data::NonInterleaved<f32>>;
    unit.set_input_callback(move |args: Args| {
        for channel in args.data.channels() {
            for sample in channel.iter().take(args.num_frames) {
                ring.push(*sample);
            }
            break;
        }
        Ok(())
    })
    .map_err(|_| FailureCode::StreamOpenFailed)?;
    unit.start().map_err(|_| FailureCode::StreamStartFailed)?;
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

pub fn run(arguments: &[String]) -> Result<(), ()> {
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
            let devices = enumerate()?;
            write_production_control(
                &mut stdout,
                capture_id,
                nonce,
                &ProductionHelperMessage::Devices { devices },
            )
            .map_err(|_| ())?;
            return Ok(());
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
            let ring = Arc::new(SpscRing::new());
            let failed = Arc::new(AtomicBool::new(false));
            let started = match backend {
                CaptureBackend::Cpal => {
                    start_cpal(device_id.as_deref(), Arc::clone(&ring), Arc::clone(&failed))
                }
                CaptureBackend::Auhal => start_auhal(device_id.as_deref(), Arc::clone(&ring)),
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
                        sequence,
                        sample_rate,
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
