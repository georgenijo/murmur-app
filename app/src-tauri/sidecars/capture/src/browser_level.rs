//! Browser output level for the optional "pause browser media while
//! dictating" setting (#835). Adapted from Hex's `browser_media.rs` and
//! `microphone_activity.rs` (MIT, Copyright (c) 2026 Christopher Langton).
//!
//! The host owns the pause/resume decision and sends the media commands; this
//! mode only hears the browsers' own output through a private, unmuted,
//! mono process tap and answers bounded level questions on stdin/stdout:
//!
//! ```text
//! worker → host  ready <in_call 0|1> <tapped 0|1>
//! host → worker  peak <1..=1000 ms>   →  peak <dBFS>|none
//! host → worker  closed               →  closed 0|1
//! EOF                                 →  teardown, exit
//! ```
//!
//! No sample leaves the IO callback: it keeps one atomic peak and a callback
//! count. Nothing identifying a process, tab, or page is ever written out.

use objc2::AnyThread;
use objc2_core_audio::{
    kAudioHardwarePropertyProcessObjectList, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioProcessPropertyBundleID,
    kAudioProcessPropertyIsRunningInput, kAudioProcessPropertyIsRunningOutput,
    kAudioProcessPropertyPID, AudioDeviceCreateIOProcIDWithBlock, AudioDeviceDestroyIOProcID,
    AudioDeviceIOProcID, AudioDeviceStart, AudioDeviceStop, AudioHardwareCreateAggregateDevice,
    AudioHardwareCreateProcessTap, AudioHardwareDestroyAggregateDevice,
    AudioHardwareDestroyProcessTap, AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize,
    AudioObjectID, AudioObjectPropertyAddress, AudioObjectPropertySelector, CATapDescription,
    CATapMuteBehavior,
};
use objc2_core_audio_types::{AudioBuffer, AudioBufferList, AudioTimeStamp};
use objc2_core_foundation::{CFRetained, CFString};
use objc2_foundation::{NSArray, NSNumber, NSString};
use std::ffi::c_void;
use std::io::{BufRead, Read, Write};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a new tap may take to deliver its first cycle.
const TAP_WARM_UP: Duration = Duration::from_millis(300);
const MAX_WINDOW_MS: u64 = 1_000;
const MAX_LINE_BYTES: usize = 32;
const MAX_SAFARI_PIDS: usize = 32;

/// When to hear the browsers. Either way the tap covers every browser
/// process, so a pause tap that stays open as the dictation's watch also
/// hears another tab or browser start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Processes {
    /// Only when one has open output (unknown counts as open): a closed
    /// output is silent, so most dictations open no tap at all.
    Playing,
    All,
}

impl Processes {
    fn includes(self, running_output: Option<bool>) -> bool {
        match self {
            Self::Playing => running_output.unwrap_or(true),
            Self::All => true,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Arguments {
    processes: Processes,
    /// `com.apple.WebKit.GPU` processes the host attributed to Safari. Every
    /// WebKit client shares that bundle ID, so the sandboxed worker cannot
    /// tell Safari from Mail by itself.
    safari_pids: Vec<i32>,
}

fn parse_arguments(arguments: &[String]) -> Option<Arguments> {
    let (mode, rest) = arguments.split_first()?;
    let processes = match mode.as_str() {
        "playing" => Processes::Playing,
        "all" => Processes::All,
        _ => return None,
    };
    if rest.len() > MAX_SAFARI_PIDS {
        return None;
    }
    let safari_pids = rest
        .iter()
        .map(|pid| pid.parse::<i32>().ok().filter(|pid| *pid > 1))
        .collect::<Option<Vec<_>>>()?;
    Some(Arguments {
        processes,
        safari_pids,
    })
}

/// Chromium-family (Chrome, Chromium, Edge, Brave, Arc, Vivaldi, Opera),
/// Firefox, or one of their audio helpers (`<bundle>.helper…`).
fn chromium_or_firefox_bundle(bundle_id: &str) -> bool {
    const BROWSERS: &[&str] = &[
        "com.google.chrome",
        "org.chromium.chromium",
        "com.microsoft.edgemac",
        "com.brave.browser",
        "company.thebrowser.browser",
        "com.vivaldi.vivaldi",
        "com.operasoftware.opera",
        "org.mozilla.firefox",
        "org.mozilla.plugincontainer",
    ];
    let bundle = bundle_id.to_ascii_lowercase();
    BROWSERS.iter().any(|prefix| {
        bundle
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
    })
}

fn browser_process(bundle_id: &str, pid: i32, safari_pids: &[i32]) -> bool {
    if bundle_id.eq_ignore_ascii_case("com.apple.WebKit.GPU") {
        return safari_pids.contains(&pid);
    }
    chromium_or_firefox_bundle(bundle_id)
}

struct ProcessObject {
    object: AudioObjectID,
    running_input: Option<bool>,
    running_output: Option<bool>,
}

fn browser_process_objects(safari_pids: &[i32]) -> Result<Vec<ProcessObject>, ()> {
    let own_pid = std::process::id() as i32;
    let objects = property_vec::<AudioObjectID>(
        kAudioObjectSystemObject as AudioObjectID,
        kAudioHardwarePropertyProcessObjectList,
    )?;
    Ok(objects
        .into_iter()
        .filter_map(|object| {
            let pid = property_value::<i32>(object, kAudioProcessPropertyPID).ok()?;
            let bundle_id = property_cf_string(object, kAudioProcessPropertyBundleID)?;
            (pid != own_pid && browser_process(&bundle_id, pid, safari_pids)).then(|| {
                let flag = |selector| {
                    property_value::<u32>(object, selector)
                        .ok()
                        .map(|running| running != 0)
                };
                ProcessObject {
                    object,
                    running_input: flag(kAudioProcessPropertyIsRunningInput),
                    running_output: flag(kAudioProcessPropertyIsRunningOutput),
                }
            })
        })
        .collect())
}

fn tap_objects(mode: Processes, processes: &[ProcessObject]) -> Vec<AudioObjectID> {
    if processes
        .iter()
        .any(|process| mode.includes(process.running_output))
    {
        processes.iter().map(|process| process.object).collect()
    } else {
        Vec::new()
    }
}

/// A browser is using the microphone (probably a call). Unknown counts as yes.
fn in_call(processes: &Result<Vec<ProcessObject>, ()>) -> bool {
    match processes {
        Ok(processes) => processes
            .iter()
            .any(|process| process.running_input.unwrap_or(true)),
        Err(()) => true,
    }
}

/// A browser process runs and every one has closed its audio output. A quit
/// browser is not a paused one: play would reach another app.
fn output_closed(safari_pids: &[i32]) -> bool {
    browser_process_objects(safari_pids).is_ok_and(|processes| {
        !processes.is_empty()
            && !processes
                .iter()
                .any(|process| Processes::Playing.includes(process.running_output))
    })
}

struct BrowserTap {
    tap: AudioObjectID,
    aggregate: AudioObjectID,
    io_proc: AudioDeviceIOProcID,
    /// The largest |sample| since the last reset, as f32 bits. Non-negative
    /// floats order the same as their bits, so `fetch_max` keeps the peak.
    peak: Arc<AtomicU32>,
    callbacks: Arc<AtomicU64>,
}

impl BrowserTap {
    fn start(objects: &[AudioObjectID]) -> Result<Self, ()> {
        let objects: Vec<_> = objects.iter().copied().map(NSNumber::new_u32).collect();
        let objects = NSArray::from_retained_slice(&objects);
        let description = unsafe {
            let description =
                CATapDescription::initMonoMixdownOfProcesses(CATapDescription::alloc(), &objects);
            description.setName(&NSString::from_str("Murmur browser level"));
            description.setPrivate(true);
            description.setMuteBehavior(CATapMuteBehavior::Unmuted);
            description
        };
        let mut source = Self {
            tap: 0,
            aggregate: 0,
            io_proc: None,
            peak: Arc::new(AtomicU32::new(0)),
            callbacks: Arc::new(AtomicU64::new(0)),
        };
        check(unsafe { AudioHardwareCreateProcessTap(Some(&description), &mut source.tap) })?;
        let tap_uid = unsafe { description.UUID().UUIDString() };
        let aggregate_uid = format!(
            "com.localdictation.browser-level.{}.{}",
            std::process::id(),
            source.tap
        );
        let properties = crate::system_audio::aggregate_properties(&tap_uid, &aggregate_uid);
        check(unsafe {
            AudioHardwareCreateAggregateDevice(&properties, NonNull::from(&mut source.aggregate))
        })?;
        let peak = Arc::clone(&source.peak);
        let callbacks = Arc::clone(&source.callbacks);
        let block = block2::RcBlock::new(
            move |_now: NonNull<AudioTimeStamp>,
                  input: NonNull<AudioBufferList>,
                  _input_time: NonNull<AudioTimeStamp>,
                  _output: NonNull<AudioBufferList>,
                  _output_time: NonNull<AudioTimeStamp>| {
                // Real-time boundary: atomics only.
                let cycle_peak = unsafe { buffer_peak(input) };
                peak.fetch_max(cycle_peak.to_bits(), Ordering::Relaxed);
                callbacks.fetch_add(1, Ordering::Relaxed);
            },
        );
        check(unsafe {
            AudioDeviceCreateIOProcIDWithBlock(
                NonNull::from(&mut source.io_proc),
                source.aggregate,
                None,
                block2::RcBlock::as_ptr(&block),
            )
        })?;
        if source.io_proc.is_none() {
            return Err(());
        }
        check(unsafe { AudioDeviceStart(source.aggregate, source.io_proc) })?;
        let warm_up = Instant::now();
        while source.callbacks.load(Ordering::Relaxed) == 0 && warm_up.elapsed() < TAP_WARM_UP {
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(source)
    }

    /// None when the tap delivered nothing during `window`.
    fn peak_dbfs(&self, window: Duration) -> Option<f32> {
        let callbacks = self.callbacks.load(Ordering::Relaxed);
        self.peak.store(0, Ordering::Relaxed);
        std::thread::sleep(window);
        if self.callbacks.load(Ordering::Relaxed) == callbacks {
            return None;
        }
        Some(dbfs(f32::from_bits(self.peak.load(Ordering::Relaxed))))
    }
}

impl Drop for BrowserTap {
    fn drop(&mut self) {
        // Reverse creation order; each identifier is torn down once.
        unsafe {
            if self.aggregate != 0 {
                if self.io_proc.is_some() {
                    let _ = AudioDeviceStop(self.aggregate, self.io_proc);
                    let _ = AudioDeviceDestroyIOProcID(self.aggregate, self.io_proc);
                }
                let _ = AudioHardwareDestroyAggregateDevice(self.aggregate);
            }
            if self.tap != 0 {
                let _ = AudioHardwareDestroyProcessTap(self.tap);
            }
        }
    }
}

fn dbfs(peak: f32) -> f32 {
    if peak > 0.0 && peak.is_finite() {
        (20.0 * peak.log10()).max(-200.0)
    } else {
        -200.0
    }
}

/// The largest |sample| in one IO cycle.
///
/// # Safety
/// `list` must point at a valid `AudioBufferList` of 32-bit float samples.
unsafe fn buffer_peak(list: NonNull<AudioBufferList>) -> f32 {
    let list = unsafe { list.as_ref() };
    let buffers: &[AudioBuffer] =
        unsafe { std::slice::from_raw_parts(list.mBuffers.as_ptr(), list.mNumberBuffers as usize) };
    buffers
        .iter()
        .filter(|buffer| !buffer.mData.is_null())
        .flat_map(|buffer| {
            let len = buffer.mDataByteSize as usize / std::mem::size_of::<f32>();
            unsafe { std::slice::from_raw_parts(buffer.mData.cast::<f32>(), len) }
        })
        .fold(0.0, |peak: f32, sample| peak.max(sample.abs()))
}

fn check(status: i32) -> Result<(), ()> {
    (status == 0).then_some(()).ok_or(())
}

#[derive(Debug, Eq, PartialEq)]
enum Request {
    Peak(Duration),
    Closed,
}

fn parse_request(line: &str) -> Option<Request> {
    match line.trim_end_matches('\n').split_once(' ') {
        Some(("peak", ms)) => ms
            .parse::<u64>()
            .ok()
            .filter(|ms| (1..=MAX_WINDOW_MS).contains(ms))
            .map(|ms| Request::Peak(Duration::from_millis(ms))),
        None if line.trim_end_matches('\n') == "closed" => Some(Request::Closed),
        _ => None,
    }
}

fn format_peak(peak: Option<f32>) -> String {
    match peak {
        Some(peak) => format!("peak {peak:.1}\n"),
        None => "peak none\n".to_string(),
    }
}

/// `--browser-level <playing|all> [safari-pid…]`. Exit 64 on bad arguments.
pub(crate) fn run(arguments: &[String]) -> i32 {
    let Some(arguments) = parse_arguments(arguments) else {
        return 64;
    };
    let processes = browser_process_objects(&arguments.safari_pids);
    let call = in_call(&processes);
    let objects = tap_objects(arguments.processes, &processes.unwrap_or_default());
    let tap = if call || objects.is_empty() || !crate::system_audio::supported() {
        None
    } else {
        BrowserTap::start(&objects).ok()
    };
    let mut stdout = std::io::stdout().lock();
    let ready = format!("ready {} {}\n", u8::from(call), u8::from(tap.is_some()));
    if stdout
        .write_all(ready.as_bytes())
        .and_then(|()| stdout.flush())
        .is_err()
    {
        return 70;
    }
    let mut stdin = std::io::stdin().lock();
    // The host keeps one tap open while a dictation holds a pause; it ends
    // the loop by closing stdin.
    loop {
        let mut line = String::new();
        match (&mut stdin)
            .take(MAX_LINE_BYTES as u64)
            .read_line(&mut line)
        {
            Ok(0) | Err(_) => return 0,
            Ok(_) => {}
        }
        let reply = match parse_request(&line) {
            Some(Request::Peak(window)) => {
                format_peak(tap.as_ref().and_then(|tap| tap.peak_dbfs(window)))
            }
            Some(Request::Closed) => {
                format!(
                    "closed {}\n",
                    u8::from(output_closed(&arguments.safari_pids))
                )
            }
            None => return 64,
        };
        if stdout
            .write_all(reply.as_bytes())
            .and_then(|()| stdout.flush())
            .is_err()
        {
            return 0;
        }
    }
}

fn property_address(selector: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}

fn property_value<T: Copy + Default>(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
) -> Result<T, ()> {
    let mut value = T::default();
    let mut size = std::mem::size_of::<T>() as u32;
    let address = property_address(selector);
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut value).cast::<c_void>(),
        )
    };
    (status == 0 && size as usize == std::mem::size_of::<T>())
        .then_some(value)
        .ok_or(())
}

fn property_vec<T: Copy + Default>(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
) -> Result<Vec<T>, ()> {
    let address = property_address(selector);
    let mut size = 0_u32;
    check(unsafe {
        AudioObjectGetPropertyDataSize(
            object,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
        )
    })?;
    if !(size as usize).is_multiple_of(std::mem::size_of::<T>()) {
        return Err(());
    }
    let mut values = vec![T::default(); size as usize / std::mem::size_of::<T>()];
    if values.is_empty() {
        return Ok(values);
    }
    check(unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new(values.as_mut_ptr().cast::<c_void>()).ok_or(())?,
        )
    })?;
    values.truncate(size as usize / std::mem::size_of::<T>());
    Ok(values)
}

fn property_cf_string(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
) -> Option<String> {
    let mut raw: *const CFString = std::ptr::null();
    let mut size = std::mem::size_of::<*const CFString>() as u32;
    let address = property_address(selector);
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut raw).cast::<c_void>(),
        )
    };
    if status != 0 || size as usize != std::mem::size_of::<*const CFString>() {
        return None;
    }
    // The HAL returns a +1 CFString for this property.
    let value = unsafe { CFRetained::from_raw(NonNull::new(raw.cast_mut())?) }.to_string();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn arguments_are_strict_and_bounded() {
        assert_eq!(
            parse_arguments(&strings(&["playing"])),
            Some(Arguments {
                processes: Processes::Playing,
                safari_pids: vec![],
            })
        );
        assert_eq!(
            parse_arguments(&strings(&["all", "4242", "77"])),
            Some(Arguments {
                processes: Processes::All,
                safari_pids: vec![4242, 77],
            })
        );
        for bad in [
            &["loud"][..],
            &[][..],
            &["all", "1"][..],
            &["all", "-5"][..],
            &["all", "safari"][..],
        ] {
            assert_eq!(parse_arguments(&strings(bad)), None, "{bad:?}");
        }
        let too_many: Vec<String> = std::iter::once("all".to_string())
            .chain((0..=MAX_SAFARI_PIDS).map(|pid| (pid + 2).to_string()))
            .collect();
        assert_eq!(parse_arguments(&too_many), None);
    }

    #[test]
    fn requests_are_strict_and_bounded() {
        assert_eq!(
            parse_request("peak 100\n"),
            Some(Request::Peak(Duration::from_millis(100)))
        );
        assert_eq!(parse_request("closed\n"), Some(Request::Closed));
        for bad in [
            "peak 0\n",
            "peak 1001\n",
            "peak\n",
            "peak x\n",
            "play\n",
            "",
        ] {
            assert_eq!(parse_request(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn browser_bundles_include_helpers_and_attributed_safari_only() {
        for bundle in [
            "com.google.Chrome",
            "com.google.Chrome.helper",
            "com.google.Chrome.canary",
            "com.brave.Browser.helper",
            "company.thebrowser.Browser.helper",
            "com.microsoft.edgemac.helper",
            "org.chromium.Chromium.helper",
            "com.vivaldi.Vivaldi.helper",
            "com.operasoftware.Opera.helper",
            "org.mozilla.plugincontainer",
        ] {
            assert!(browser_process(bundle, 10, &[]), "{bundle}");
        }
        for bundle in [
            "com.spotify.client",
            "com.apple.Music",
            "com.google.Chromecast",
            "com.localdictation.app",
        ] {
            assert!(!browser_process(bundle, 10, &[]), "{bundle}");
        }
        assert!(browser_process("com.apple.WebKit.GPU", 10, &[10]));
        assert!(!browser_process("com.apple.WebKit.GPU", 11, &[10]));
    }

    #[test]
    fn a_pause_taps_only_browsers_with_open_output() {
        assert!(Processes::Playing.includes(Some(true)));
        assert!(!Processes::Playing.includes(Some(false)));
        assert!(Processes::Playing.includes(None));
        assert!(Processes::All.includes(Some(false)));
    }

    #[test]
    fn one_open_output_taps_every_browser_and_none_taps_nothing() {
        let process = |object, running_output| ProcessObject {
            object,
            running_input: Some(false),
            running_output,
        };
        let closed = [process(1, Some(false)), process(2, Some(false))];
        assert!(tap_objects(Processes::Playing, &closed).is_empty());
        assert_eq!(tap_objects(Processes::All, &closed), vec![1, 2]);
        let one_open = [process(1, Some(false)), process(2, Some(true))];
        assert_eq!(tap_objects(Processes::Playing, &one_open), vec![1, 2]);
        assert!(tap_objects(Processes::Playing, &[]).is_empty());
    }

    #[test]
    fn unknown_microphone_use_counts_as_a_call() {
        assert!(in_call(&Err(())));
        assert!(!in_call(&Ok(vec![])));
        let process = |running_input| ProcessObject {
            object: 1,
            running_input,
            running_output: Some(true),
        };
        assert!(in_call(&Ok(vec![process(None)])));
        assert!(in_call(&Ok(vec![process(Some(true))])));
        assert!(!in_call(&Ok(vec![process(Some(false))])));
    }

    #[test]
    fn levels_are_bounded_and_content_free() {
        assert_eq!(dbfs(0.0), -200.0);
        assert_eq!(dbfs(f32::NAN), -200.0);
        assert_eq!(dbfs(1.0), 0.0);
        assert!((dbfs(0.1) + 20.0).abs() < 0.01);
        assert_eq!(format_peak(Some(-25.43)), "peak -25.4\n");
        assert_eq!(format_peak(None), "peak none\n");
    }
}
