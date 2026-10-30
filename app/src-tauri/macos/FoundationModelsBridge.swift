// In-process bridge from the homophone context pass (issue #833) to Apple's
// on-device FoundationModels language model. Compiled by build.rs into a
// static library. Nothing here logs, persists, or transmits prompt or answer
// text; the Rust caller owns every privacy and acceptance decision.
//
// When the SDK lacks FoundationModels, or the running OS predates macOS 26,
// every entry point reports "unavailable" and the caller keeps deterministic
// text.

import Foundation
#if canImport(FoundationModels) && !MURMUR_FM_STUB
import FoundationModels
#endif

private let statusOk: Int32 = 0
private let statusUnavailable: Int32 = 1
private let statusTimeout: Int32 = 2
private let statusFailed: Int32 = 3

private final class AnswerBox: @unchecked Sendable {
    private let lock = NSLock()
    private var answer: String?
    private var status = statusFailed

    func finish(_ value: String?, status: Int32) {
        lock.lock()
        answer = value
        self.status = status
        lock.unlock()
    }

    func take() -> (String?, Int32) {
        lock.lock()
        defer { lock.unlock() }
        return (answer, status)
    }
}

// Only one inference may remain alive, including after timeout cancellation.
// A replacement refuses immediately until the previous task actually exits.
private final class InferenceSlot: @unchecked Sendable {
    static let shared = InferenceSlot()
    private let lock = NSLock()
    private var busy = false

    func claim() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard !busy else { return false }
        busy = true
        return true
    }

    func release() {
        lock.lock()
        busy = false
        lock.unlock()
    }
}

/// Runs one bounded request. On success `*out` receives a malloc'd UTF-8
/// answer that the caller must release with `murmur_fm_free`.
@_cdecl("murmur_fm_choose")
public func murmur_fm_choose(
    _ instructions: UnsafePointer<CChar>,
    _ prompt: UnsafePointer<CChar>,
    _ timeoutMs: UInt32,
    _ maxTokens: UInt32,
    _ out: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>
) -> Int32 {
    let deadline = DispatchTime.now() + .milliseconds(Int(timeoutMs))
    out.pointee = nil
    #if canImport(FoundationModels) && !MURMUR_FM_STUB
    if #available(macOS 26.0, *) {
        guard InferenceSlot.shared.claim() else { return statusFailed }
        let instructionText = String(cString: instructions)
        let promptText = String(cString: prompt)
        let box = AnswerBox()
        let done = DispatchSemaphore(value: 0)
        let task = Task.detached {
            defer { InferenceSlot.shared.release(); done.signal() }
            do {
                try Task.checkCancellation()
                // Availability can consult system services too: keep it inside
                // the same deadline as session creation and generation.
                guard case .available = SystemLanguageModel.default.availability else {
                    box.finish(nil, status: statusUnavailable)
                    return
                }
                try Task.checkCancellation()
                let session = LanguageModelSession(instructions: instructionText)
                let response = try await session.respond(
                    to: promptText,
                    options: GenerationOptions(
                        sampling: .greedy,
                        maximumResponseTokens: Int(maxTokens)
                    )
                )
                box.finish(response.content, status: statusOk)
            } catch {
                box.finish(nil, status: statusFailed)
            }
        }
        if done.wait(timeout: deadline) == .timedOut {
            task.cancel()
            return statusTimeout
        }
        let (answer, status) = box.take()
        guard status == statusOk else { return status }
        guard let answer, let copy = strdup(answer) else {
            return statusFailed
        }
        out.pointee = copy
        return statusOk
    }
    #endif
    return statusUnavailable
}

@_cdecl("murmur_fm_free")
public func murmur_fm_free(_ pointer: UnsafeMutablePointer<CChar>?) {
    free(pointer)
}
