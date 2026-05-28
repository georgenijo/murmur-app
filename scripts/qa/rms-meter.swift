// CoreAudio process-tap RMS meter (macOS 14.4+).
//
// Taps the global system audio OUTPUT (unmuted — does not affect what you hear)
// and prints the output RMS level ~10x/second:
//
//     HH:MM:SS.SSS rms=<linear> db=<dBFS> n=<samples>
//
// Used by audio-duck-check.sh to detect "ducking" — when bringing an app to the
// foreground attenuates other audio (issue #177). Build:
//
//     swiftc -O rms-meter.swift -o rms-meter \
//         -framework CoreAudio -framework AVFoundation -framework Foundation
//
// First run triggers a one-time audio-recording permission prompt for the terminal.

import CoreAudio
import AVFoundation
import Foundation

func check(_ status: OSStatus, _ msg: String) {
    if status != noErr {
        FileHandle.standardError.write("ERR \(msg): \(status)\n".data(using: .utf8)!)
        exit(1)
    }
}

let desc = CATapDescription(stereoGlobalTapButExcludeProcesses: [])
desc.name = "rms-tap"
desc.isPrivate = true
desc.muteBehavior = .unmuted

var tapID = AudioObjectID(kAudioObjectUnknown)
check(AudioHardwareCreateProcessTap(desc, &tapID), "create tap")
let tapUUID = desc.uuid.uuidString

let aggUID = "rms-agg-\(UUID().uuidString)"
let aggDict: [String: Any] = [
    kAudioAggregateDeviceNameKey as String: "rms-agg",
    kAudioAggregateDeviceUIDKey as String: aggUID,
    kAudioAggregateDeviceIsPrivateKey as String: true,
    kAudioAggregateDeviceIsStackedKey as String: false,
    kAudioAggregateDeviceTapAutoStartKey as String: true,
    kAudioAggregateDeviceTapListKey as String: [
        [
            kAudioSubTapUIDKey as String: tapUUID,
            kAudioSubTapDriftCompensationKey as String: false,
        ]
    ],
]
var aggID = AudioObjectID(kAudioObjectUnknown)
check(AudioHardwareCreateAggregateDevice(aggDict as CFDictionary, &aggID), "create agg")

var sumSq = 0.0
var sampleCount = 0
let lock = NSLock()

let ioBlock: AudioDeviceIOBlock = { (_, inInputData, _, _, _) in
    let abl = UnsafeMutableAudioBufferListPointer(
        UnsafeMutablePointer(mutating: inInputData))
    var ls = 0.0
    var n = 0
    for buf in abl {
        guard let mData = buf.mData else { continue }
        let count = Int(buf.mDataByteSize) / MemoryLayout<Float32>.size
        let ptr = mData.assumingMemoryBound(to: Float32.self)
        for i in 0..<count {
            let v = Double(ptr[i])
            ls += v * v
            n += 1
        }
    }
    lock.lock(); sumSq += ls; sampleCount += n; lock.unlock()
}

var ioProcID: AudioDeviceIOProcID?
check(AudioDeviceCreateIOProcIDWithBlock(&ioProcID, aggID, nil, ioBlock), "create ioproc")
check(AudioDeviceStart(aggID, ioProcID), "start")

func cleanup() {
    if let p = ioProcID { AudioDeviceStop(aggID, p); AudioDeviceDestroyIOProcID(aggID, p) }
    AudioHardwareDestroyAggregateDevice(aggID)
    AudioHardwareDestroyProcessTap(tapID)
}

let sig = DispatchSource.makeSignalSource(signal: SIGINT, queue: .main)
sig.setEventHandler { cleanup(); exit(0) }
sig.resume()
signal(SIGINT, SIG_IGN)

let fmt = DateFormatter()
fmt.dateFormat = "HH:mm:ss.SSS"
let timer = DispatchSource.makeTimerSource(queue: .global())
timer.schedule(deadline: .now(), repeating: .milliseconds(100))
timer.setEventHandler {
    lock.lock(); let s = sumSq; let c = sampleCount; sumSq = 0; sampleCount = 0; lock.unlock()
    let rms = c > 0 ? sqrt(s / Double(c)) : 0
    let db = rms > 0 ? 20 * log10(rms) : -120
    print(String(format: "%@ rms=%.5f db=%.1f n=%d", fmt.string(from: Date()), rms, db, c))
    fflush(stdout)
}
timer.resume()

FileHandle.standardError.write("rms-tap running (tap=\(tapID) agg=\(aggID))\n".data(using: .utf8)!)
RunLoop.main.run()
