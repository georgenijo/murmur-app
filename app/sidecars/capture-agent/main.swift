import Foundation
import Security
import XPC
import Darwin

private func currentStaticCode() -> SecStaticCode? {
    var runningCode: SecCode?
    guard SecCodeCopySelf([], &runningCode) == errSecSuccess,
          let runningCode else {
        return nil
    }
    var staticCode: SecStaticCode?
    guard SecCodeCopyStaticCode(runningCode, [], &staticCode) == errSecSuccess,
          let staticCode else {
        return nil
    }
    return staticCode
}

private func currentExecutableURL() -> URL? {
    guard let staticCode = currentStaticCode() else {
        return nil
    }
    var executableURL: CFURL?
    guard SecCodeCopyPath(staticCode, [], &executableURL) == errSecSuccess,
          let executableURL else {
        return nil
    }
    return executableURL as URL
}

private func currentSigningTeamID() -> String? {
    guard let staticCode = currentStaticCode() else {
        return nil
    }
    var information: CFDictionary?
    let signingInformation = SecCSFlags(rawValue: 1 << 1)
    guard SecCodeCopySigningInformation(
        staticCode,
        signingInformation,
        &information
    ) == errSecSuccess,
    let values = information as? [String: Any] else {
        return nil
    }
    return values[kSecCodeInfoTeamIdentifier as String] as? String
}

private let agentIdentifier = "com.localdictation.capture-agent"
private let captureWorkerIdentifier = "com.localdictation.capture-worker"
private let signingTeamID = currentSigningTeamID() ?? ""
private let machServiceName = "com.localdictation.capture-agent.xpc"
private let protocolName = "murmur.capture_probe"
private let protocolVersion: Int = 1
private let recoveryTTL: TimeInterval = 30
private let maxCanaries: UInt64 = 4_096
private let syntheticFixture = "seq-v1"
private let syntheticFixtureChunks: UInt64 = 64
private let syntheticFixtureDigest =
    "9fda676f94adbf56e31e91462c702dcda9fcf989eece435876a28778782abfd3"
private let peerRequirement =
    "identifier \"\(agentIdentifier)\" and anchor apple generic and " +
    "certificate leaf[subject.OU] = \"\(signingTeamID)\""

private func disableCoreDumps() {
    var limit = rlimit(rlim_cur: 0, rlim_max: 0)
    _ = setrlimit(RLIMIT_CORE, &limit)
}

private func jsonData(_ value: [String: Any]) -> Data? {
    try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
}

private func jsonLine(_ value: [String: Any]) {
    guard let data = jsonData(value), let text = String(data: data, encoding: .utf8) else {
        return
    }
    FileHandle.standardOutput.write(Data((text + "\n").utf8))
}

private func exactUInt64(_ value: Any?) -> UInt64? {
    guard let number = value as? NSNumber,
          CFGetTypeID(number) != CFBooleanGetTypeID() else {
        return nil
    }
    let floating = number.doubleValue
    guard floating.isFinite,
          floating >= 0,
          floating.rounded(.towardZero) == floating,
          floating <= Double(UInt64.max) else {
        return nil
    }
    return number.uint64Value
}

private func validateCaptureWorker(at executable: URL) -> Bool {
    var staticCode: SecStaticCode?
    guard SecStaticCodeCreateWithPath(executable as CFURL, [], &staticCode) == errSecSuccess,
          let staticCode else {
        return false
    }
    let requirementText =
        "identifier \"\(captureWorkerIdentifier)\" and anchor apple generic and " +
        "certificate leaf[subject.OU] = \"\(signingTeamID)\""
    var requirement: SecRequirement?
    guard SecRequirementCreateWithString(
        requirementText as CFString,
        [],
        &requirement
    ) == errSecSuccess, let requirement else {
        return false
    }
    // kSecCSStrictValidate is not imported as a Swift member by every SDK.
    let strictValidate = SecCSFlags(rawValue: 1 << 4)
    return SecStaticCodeCheckValidity(staticCode, strictValidate, requirement) == errSecSuccess
}

private func xpcString(_ object: xpc_object_t, _ key: String) -> String? {
    key.withCString { rawKey in
        guard let value = xpc_dictionary_get_string(object, rawKey) else { return nil }
        return String(cString: value)
    }
}

private func setXPCString(_ object: xpc_object_t, _ key: String, _ value: String) {
    key.withCString { rawKey in
        value.withCString { rawValue in
            xpc_dictionary_set_string(object, rawKey, rawValue)
        }
    }
}

private func setXPCUInt64(_ object: xpc_object_t, _ key: String, _ value: UInt64) {
    key.withCString { rawKey in
        xpc_dictionary_set_uint64(object, rawKey, value)
    }
}

private func setXPCBool(_ object: xpc_object_t, _ key: String, _ value: Bool) {
    key.withCString { rawKey in
        xpc_dictionary_set_bool(object, rawKey, value)
    }
}

private func dictionary(from object: xpc_object_t) -> [String: Any]? {
    var result: [String: Any] = [:]
    for key in [
        "outcome", "service_status", "agent_instance", "worker_termination", "failure",
        "claim_id", "synthetic_fixture", "synthetic_digest",
    ] {
        let accepted = key.withCString { rawKey -> Bool in
            guard let value = xpc_dictionary_get_value(object, rawKey) else {
                return true
            }
            guard xpc_get_type(value) == XPC_TYPE_STRING,
                  let string = xpc_dictionary_get_string(object, rawKey) else {
                return false
            }
            result[key] = String(cString: string)
            return true
        }
        if !accepted { return nil }
    }
    for key in [
        "schema_version", "generation", "agent_pid", "worker_pid",
        "synthetic_canary_count", "first_callback_ms", "stop_elapsed_ms",
        "recovery_ttl_ms", "synthetic_first_sequence", "synthetic_last_sequence",
        "exit_signal",
    ] {
        let accepted = key.withCString { rawKey -> Bool in
            guard let value = xpc_dictionary_get_value(object, rawKey) else {
                return true
            }
            guard xpc_get_type(value) == XPC_TYPE_UINT64 else { return false }
            result[key] = xpc_dictionary_get_uint64(object, rawKey)
            return true
        }
        if !accepted { return nil }
    }
    for key in [
        "audio_content_retained", "recovered", "agent_survived",
        "worker_exited", "process_group_empty", "exact_once", "synthetic_complete",
    ] {
        let accepted = key.withCString { rawKey -> Bool in
            guard let value = xpc_dictionary_get_value(object, rawKey) else {
                return true
            }
            guard xpc_get_type(value) == XPC_TYPE_BOOL else { return false }
            result[key] = xpc_dictionary_get_bool(object, rawKey)
            return true
        }
        if !accepted { return nil }
    }
    guard xpc_dictionary_get_count(object) == result.count else { return nil }
    return result
}

private func readExactly(_ handle: FileHandle, count: Int) -> Data? {
    var data = Data()
    while data.count < count {
        guard let chunk = try? handle.read(upToCount: count - data.count),
              !chunk.isEmpty else {
            return nil
        }
        data.append(chunk)
    }
    return data
}

private func readFrame(_ handle: FileHandle) -> [String: Any]? {
    guard let header = readExactly(handle, count: 4) else { return nil }
    let length = header.withUnsafeBytes { raw -> UInt32 in
        raw.loadUnaligned(as: UInt32.self).bigEndian
    }
    guard length > 0, length <= 4_096,
          let body = readExactly(handle, count: Int(length)),
          let object = try? JSONSerialization.jsonObject(with: body),
          let dictionary = object as? [String: Any] else {
        return nil
    }
    return dictionary
}

private func writeFrame(_ value: [String: Any], to handle: FileHandle) -> Bool {
    guard let body = jsonData(value), !body.isEmpty, body.count <= 4_096 else {
        return false
    }
    var length = UInt32(body.count).bigEndian
    var framed = Data(bytes: &length, count: 4)
    framed.append(body)
    do {
        try handle.write(contentsOf: framed)
        return true
    } catch {
        return false
    }
}

private enum WorkerProtocolState: Equatable {
    case awaitingEnumeration
    case awaitingStreamOpen
    case awaitingReady
    case awaitingFirstCallbackPhase
    case awaitingFirstCallback
    case awaitingActive
    case active
    case failed
    case stopping
    case stopped
}

private enum WorkerEvent {
    case phase
    case ready
    case firstCallback(UInt64)
    case health
    case syntheticChunk(UInt64)
    case failure(String)
    case stopped
}

private enum WorkerMode: Equatable {
    case microphone
    case synthetic
    case syntheticIgnoreCancel

    var isSynthetic: Bool {
        self != .microphone
    }
}

private struct WorkerEvidence {
    let canaryCount: UInt64
    let firstCallbackMS: UInt64
    let firstSyntheticSequence: UInt64?
    let lastSyntheticSequence: UInt64?
}

private struct WorkerTermination {
    let kind: String
    let elapsedMS: UInt64
    let workerExited: Bool
    let processGroupEmpty: Bool
    let exitSignal: UInt64
}

private final class WorkerSession {
    private let mode: WorkerMode
    private let process = Process()
    private let input = Pipe()
    private let output = Pipe()
    private let nonce = "agent-\(UUID().uuidString)"
    private let readerQueue = DispatchQueue(label: "com.localdictation.capture-agent.worker-reader")
    private let completionQueue = DispatchQueue(label: "com.localdictation.capture-agent.worker-completion")
    private let stateLock = NSLock()
    private var completion: ((WorkerTermination) -> Void)?
    private var stopStarted = DispatchTime.now()
    private var stoppedFrameSeen = false
    private var terminationSeen = false
    private var readerCompleted = false
    private var completionDelivered = false
    private var cancelSent = false
    private var hardKillSent = false
    private var processGroupOwned = false
    private var processGroupEmpty = false
    private var processGroupCheckCompleted = false
    private var exitSignal: UInt64 = 0
    private var protocolState = WorkerProtocolState.awaitingEnumeration
    private var nextSyntheticSequence: UInt64 = 0
    private var lastHealthRank = 0
    private var evidenceCanaryCount: UInt64 = 0
    private var evidenceFirstCallbackMS: UInt64 = 0
    private var evidenceFirstSyntheticSequence: UInt64?
    private var evidenceLastSyntheticSequence: UInt64?

    var onReady: ((UInt64) -> Void)?
    var onCanary: (() -> Void)?
    var onSyntheticChunk: ((UInt64) -> Void)?
    var onFailure: ((String) -> Void)?

    var pid: UInt64 {
        UInt64(process.processIdentifier)
    }

    init(mode: WorkerMode) {
        self.mode = mode
    }

    func start() throws {
        guard let executable = currentExecutableURL()?
            .deletingLastPathComponent()
            .appendingPathComponent("murmur-capture-worker") else {
            throw NSError(domain: agentIdentifier, code: 2)
        }
        guard validateCaptureWorker(at: executable) else {
            throw NSError(domain: agentIdentifier, code: 2)
        }
        process.executableURL = executable
        switch mode {
        case .microphone:
            process.arguments = []
        case .synthetic:
            process.arguments = ["--synthetic-fixture", syntheticFixture]
        case .syntheticIgnoreCancel:
            process.arguments = [
                "--synthetic-fixture", syntheticFixture, "--ignore-cancel",
            ]
        }
        process.currentDirectoryURL = URL(fileURLWithPath: "/")
        process.standardInput = input
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        process.environment = [:]
        process.terminationHandler = { [weak self] _ in
            self?.processTerminated()
        }
        try process.run()
        let groupDeadline = DispatchTime.now() + .seconds(1)
        while process.isRunning,
              Darwin.getpgid(process.processIdentifier) != process.processIdentifier,
              DispatchTime.now() < groupDeadline {
            usleep(1_000)
        }
        guard process.isRunning,
              Darwin.getpgid(process.processIdentifier) == process.processIdentifier else {
            _ = Darwin.kill(process.processIdentifier, SIGKILL)
            process.waitUntilExit()
            throw NSError(domain: agentIdentifier, code: 3)
        }
        stateLock.lock()
        processGroupOwned = true
        stateLock.unlock()

        let hello: [String: Any] = [
            "type": "hello",
            "protocol": protocolName,
            "version": protocolVersion,
            "sessionNonce": nonce,
        ]
        guard writeFrame(hello, to: input.fileHandleForWriting) else {
            process.terminate()
            throw NSError(domain: agentIdentifier, code: 1)
        }

        readerQueue.async { [weak self] in
            self?.readLoop()
        }
    }

    private func validEnvelope(_ frame: [String: Any], keys: Set<String>) -> Bool {
        Set(frame.keys) == keys
            && frame["protocol"] as? String == protocolName
            && exactUInt64(frame["version"]) == UInt64(protocolVersion)
            && frame["sessionNonce"] as? String == nonce
    }

    private func readLoop() {
        var expectedEOF = false
        workerFrames: while let frame = readFrame(output.fileHandleForReading) {
            guard let event = accept(frame) else {
                onFailure?("protocol")
                break workerFrames
            }
            switch event {
            case .phase, .ready:
                continue
            case .firstCallback(let latency):
                if mode == .microphone {
                    onCanary?()
                }
                onReady?(latency)
            case .health:
                onCanary?()
            case .syntheticChunk(let sequence):
                onSyntheticChunk?(sequence)
            case .failure(let code):
                onFailure?(code)
                break workerFrames
            case .stopped:
                continue
            }
        }
        stateLock.lock()
        readerCompleted = true
        expectedEOF = cancelSent || terminationSeen
        stateLock.unlock()
        if !expectedEOF {
            onFailure?("worker_stdout_eof")
        }
        deliverCompletionIfPossible()
    }

    private func accept(_ frame: [String: Any]) -> WorkerEvent? {
        guard let type = frame["type"] as? String else { return nil }
        stateLock.lock()
        defer { stateLock.unlock() }
        switch type {
        case "phase":
            guard validEnvelope(
                frame,
                keys: ["type", "protocol", "version", "sessionNonce", "phase"]
            ), let phase = frame["phase"] as? String else {
                return nil
            }
            switch (protocolState, phase) {
            case (.awaitingEnumeration, "enumeration"):
                protocolState = .awaitingStreamOpen
            case (.awaitingStreamOpen, "streamOpen"):
                protocolState = .awaitingReady
            case (.awaitingFirstCallbackPhase, "awaitingFirstCallback"):
                protocolState = .awaitingFirstCallback
            case (.awaitingActive, "active"):
                protocolState = .active
            case (_, "stopping")
                where cancelSent
                    && protocolState != .failed
                    && protocolState != .stopping
                    && protocolState != .stopped:
                protocolState = .stopping
            default:
                return nil
            }
            return .phase
        case "ready":
            guard protocolState == .awaitingReady,
                  validEnvelope(
                      frame,
                      keys: ["type", "protocol", "version", "sessionNonce"]
                  ) else {
                return nil
            }
            protocolState = .awaitingFirstCallbackPhase
            return .ready
        case "firstCallback":
            guard protocolState == .awaitingFirstCallback,
                  validEnvelope(
                      frame,
                      keys: [
                          "type", "protocol", "version", "sessionNonce",
                          "callbackLatencyMs",
                      ]
                  ), let latency = exactUInt64(frame["callbackLatencyMs"]),
                  latency <= 60_000 else {
                return nil
            }
            protocolState = .awaitingActive
            evidenceFirstCallbackMS = latency
            if mode == .microphone {
                evidenceCanaryCount = min(maxCanaries, evidenceCanaryCount &+ 1)
            }
            return .firstCallback(latency)
        case "callbackHealth":
            guard protocolState == .active,
                  validEnvelope(
                      frame,
                      keys: [
                          "type", "protocol", "version", "sessionNonce",
                          "callbackCountBucket",
                      ]
                  ), let bucket = frame["callbackCountBucket"] as? String,
                  let rank = ["0", "le10", "le100", "le1k", "gt1k"].firstIndex(of: bucket),
                  rank >= lastHealthRank else {
                return nil
            }
            lastHealthRank = rank
            evidenceCanaryCount = min(maxCanaries, evidenceCanaryCount &+ 1)
            return .health
        case "syntheticChunk":
            guard mode.isSynthetic,
                  protocolState == .active,
                  validEnvelope(
                      frame,
                      keys: [
                          "type", "protocol", "version", "sessionNonce",
                          "fixture", "fixtureDigest", "sequence",
                      ]
                  ),
                  frame["fixture"] as? String == syntheticFixture,
                  frame["fixtureDigest"] as? String == syntheticFixtureDigest,
                  let sequence = exactUInt64(frame["sequence"]),
                  sequence == nextSyntheticSequence,
                  sequence < syntheticFixtureChunks else {
                return nil
            }
            nextSyntheticSequence &+= 1
            if evidenceFirstSyntheticSequence == nil {
                evidenceFirstSyntheticSequence = sequence
            }
            evidenceLastSyntheticSequence = sequence
            evidenceCanaryCount = min(maxCanaries, evidenceCanaryCount &+ 1)
            return .syntheticChunk(sequence)
        case "failure":
            guard protocolState != .failed,
                  protocolState != .stopping,
                  protocolState != .stopped,
                  validEnvelope(
                      frame,
                      keys: ["type", "protocol", "version", "sessionNonce", "code"]
                  ), let code = frame["code"] as? String,
                  [
                      "permissionDenied", "noInputDevice", "enumerationFailed",
                      "configurationFailed", "streamOpenFailed", "streamStartFailed",
                      "streamError", "callbackStalled", "invalidMessage", "internal",
                  ].contains(code) else {
                return nil
            }
            protocolState = .failed
            return .failure(code)
        case "stopped":
            guard cancelSent,
                  protocolState == .stopping,
                  validEnvelope(
                      frame,
                      keys: ["type", "protocol", "version", "sessionNonce"]
                  ) else {
                return nil
            }
            protocolState = .stopped
            stoppedFrameSeen = true
            return .stopped
        default:
            return nil
        }
    }

    func evidence() -> WorkerEvidence {
        stateLock.lock()
        defer { stateLock.unlock() }
        return WorkerEvidence(
            canaryCount: evidenceCanaryCount,
            firstCallbackMS: evidenceFirstCallbackMS,
            firstSyntheticSequence: evidenceFirstSyntheticSequence,
            lastSyntheticSequence: evidenceLastSyntheticSequence
        )
    }

    func stop(completion: @escaping (WorkerTermination) -> Void) {
        stateLock.lock()
        self.completion = completion
        stopStarted = DispatchTime.now()
        cancelSent = true
        stateLock.unlock()
        let cancel: [String: Any] = [
            "type": "cancel",
            "protocol": protocolName,
            "version": protocolVersion,
            "sessionNonce": nonce,
        ]
        _ = writeFrame(cancel, to: input.fileHandleForWriting)
        completionQueue.asyncAfter(deadline: .now() + .milliseconds(250)) { [weak self] in
            guard let self, self.process.isRunning else { return }
            self.stateLock.lock()
            self.hardKillSent = true
            let ownsGroup = self.processGroupOwned
            self.stateLock.unlock()
            let pid = self.process.processIdentifier
            if !ownsGroup || Darwin.kill(-pid, SIGKILL) != 0 {
                _ = Darwin.kill(pid, SIGKILL)
            }
        }
        deliverCompletionIfPossible()
    }

    private func processTerminated() {
        stateLock.lock()
        terminationSeen = true
        if process.terminationReason == .uncaughtSignal {
            exitSignal = UInt64(process.terminationStatus)
        }
        let wasExpected = completion != nil
        stateLock.unlock()
        if !wasExpected {
            onFailure?("worker_exited")
        }
        completionQueue.async { [weak self] in
            self?.confirmProcessGroupEmpty()
        }
    }

    private func confirmProcessGroupEmpty() {
        let deadline = DispatchTime.now() + .seconds(2)
        while true {
            stateLock.lock()
            let ownsGroup = processGroupOwned
            stateLock.unlock()
            let probe = ownsGroup ? Darwin.kill(-process.processIdentifier, 0) : -1
            if !ownsGroup || (probe != 0 && Darwin.errno == ESRCH) {
                stateLock.lock()
                processGroupEmpty = true
                stateLock.unlock()
                break
            }
            _ = Darwin.kill(-process.processIdentifier, SIGKILL)
            if DispatchTime.now() >= deadline {
                break
            }
            usleep(5_000)
        }
        stateLock.lock()
        processGroupCheckCompleted = true
        stateLock.unlock()
        deliverCompletionIfPossible()
    }

    private func deliverCompletionIfPossible() {
        stateLock.lock()
        guard terminationSeen,
              readerCompleted,
              processGroupCheckCompleted,
              !completionDelivered,
              let callback = completion else {
            stateLock.unlock()
            return
        }
        completionDelivered = true
        let elapsed = DispatchTime.now().uptimeNanoseconds - stopStarted.uptimeNanoseconds
        let kind = hardKillSent
            ? (exitSignal == UInt64(SIGKILL) ? "hard_kill" : "kill_unconfirmed")
            : (stoppedFrameSeen ? "cooperative" : "exited")
        let termination = WorkerTermination(
            kind: kind,
            elapsedMS: elapsed / 1_000_000,
            workerExited: terminationSeen,
            processGroupEmpty: processGroupEmpty,
            exitSignal: exitSignal
        )
        stateLock.unlock()
        completionQueue.async {
            callback(termination)
        }
    }
}

private struct PendingRecovery {
    let generation: UInt64
    let agentPID: UInt64
    let agentBootNonce: String
    let workerPID: UInt64
    let syntheticCanaryCount: UInt64
    let firstCallbackMS: UInt64
    let workerTermination: String
    let workerExited: Bool
    let processGroupEmpty: Bool
    let exitSignal: UInt64
    let stopElapsedMS: UInt64
    let expiresAt: DispatchTime
    let syntheticFixture: String?
    let syntheticDigest: String?
    let syntheticFirstSequence: UInt64?
    let syntheticLastSequence: UInt64?
    var claimID: String?
    var claimOwner: String?
}

private struct AcknowledgedTombstone {
    let recovery: PendingRecovery
}

private final class AgentState {
    private let queue = DispatchQueue(label: "com.localdictation.capture-agent.state")
    // Opaque observation-only fingerprint. It is never used for authorization.
    private let instanceFingerprint = UUID().uuidString
    private var generation: UInt64 = 0
    private var activeLeaseID: String?
    private var worker: WorkerSession?
    private var canaryCount: UInt64 = 0
    private var firstCallbackMS: UInt64 = 0
    private var startReplySent = false
    private var pending: PendingRecovery?
    private var acknowledged: AcknowledgedTombstone?
    private var expiredGeneration: UInt64?
    private var activeMode = WorkerMode.microphone
    private var firstSyntheticSequence: UInt64?
    private var lastSyntheticSequence: UInt64?

    func handle(peerID: String, peer: xpc_connection_t, request: xpc_object_t) {
        queue.async {
            guard xpc_dictionary_get_count(request) == 1 else {
                self.reply(
                    peer: peer,
                    request: request,
                    fields: ["outcome": "invalid_command", "audio_content_retained": false]
                )
                return
            }
            switch xpcString(request, "command") {
            case "start":
                self.start(
                    peerID: peerID,
                    peer: peer,
                    request: request,
                    mode: .microphone
                )
            case "start_synthetic":
                self.start(
                    peerID: peerID,
                    peer: peer,
                    request: request,
                    mode: .synthetic
                )
            case "start_synthetic_fault":
                self.start(
                    peerID: peerID,
                    peer: peer,
                    request: request,
                    mode: .syntheticIgnoreCancel
                )
            case "stop":
                self.stop(peerID: peerID, peer: peer, request: request)
            case "recover":
                self.recover(peerID: peerID, peer: peer, request: request)
            case "status":
                self.status(peer: peer, request: request)
            default:
                if let command = xpcString(request, "command"),
                   command.hasPrefix("ack:"),
                   command.utf8.count <= 132 {
                    self.ack(
                        peerID: peerID,
                        claimID: String(command.dropFirst(4)),
                        peer: peer,
                        request: request
                    )
                } else {
                    self.reply(
                        peer: peer,
                        request: request,
                        fields: ["outcome": "invalid_command", "audio_content_retained": false]
                    )
                }
            }
        }
    }

    func disconnected(peerID: String) {
        queue.async {
            if self.activeLeaseID == peerID {
                self.interruptActive()
                return
            }
            if self.pending?.claimOwner == peerID {
                self.pending?.claimOwner = nil
                self.pending?.claimID = nil
            }
        }
    }

    private func start(
        peerID: String,
        peer: xpc_connection_t,
        request: xpc_object_t,
        mode: WorkerMode
    ) {
        expirePending()
        guard activeLeaseID == nil, worker == nil, pending == nil else {
            reply(
                peer: peer,
                request: request,
                fields: ["outcome": "busy", "audio_content_retained": false]
            )
            return
        }
        generation &+= 1
        acknowledged = nil
        expiredGeneration = nil
        canaryCount = 0
        firstCallbackMS = 0
        startReplySent = false
        activeMode = mode
        firstSyntheticSequence = nil
        lastSyntheticSequence = nil
        activeLeaseID = peerID

        let nextWorker = WorkerSession(mode: mode)
        worker = nextWorker
        nextWorker.onCanary = { [weak self] in
            self?.queue.async {
                guard let self, self.activeLeaseID == peerID else { return }
                self.canaryCount = min(maxCanaries, self.canaryCount &+ 1)
            }
        }
        nextWorker.onReady = { [weak self] latency in
            self?.queue.async {
                guard let self,
                      self.activeLeaseID == peerID,
                      !self.startReplySent else { return }
                self.startReplySent = true
                self.firstCallbackMS = latency
                self.reply(
                    peer: peer,
                    request: request,
                    fields: [
                        "outcome": "ready",
                        "generation": self.generation,
                        "agent_pid": UInt64(getpid()),
                        "agent_instance": self.instanceFingerprint,
                        "worker_pid": nextWorker.pid,
                        "synthetic_canary_count": self.canaryCount,
                        "first_callback_ms": latency,
                        "audio_content_retained": false,
                    ]
                )
            }
        }
        nextWorker.onSyntheticChunk = { [weak self] sequence in
            self?.queue.async {
                guard let self,
                      self.activeLeaseID == peerID,
                      self.activeMode.isSynthetic else { return }
                if self.firstSyntheticSequence == nil {
                    self.firstSyntheticSequence = sequence
                }
                self.lastSyntheticSequence = sequence
                self.canaryCount = min(maxCanaries, self.canaryCount &+ 1)
            }
        }
        nextWorker.onFailure = { [weak self] failure in
            self?.queue.async {
                guard let self, self.activeLeaseID == peerID else { return }
                if !self.startReplySent {
                    self.startReplySent = true
                    self.reply(
                        peer: peer,
                        request: request,
                        fields: [
                            "outcome": "worker_failed",
                            "failure": failure,
                            "audio_content_retained": false,
                        ]
                    )
                }
                self.interruptActive()
            }
        }

        do {
            try nextWorker.start()
        } catch {
            worker = nil
            activeLeaseID = nil
            let failure: String
            let launchError = error as NSError
            if launchError.domain == agentIdentifier && launchError.code == 2 {
                failure = "worker_signature_invalid"
            } else if launchError.domain == agentIdentifier && launchError.code == 3 {
                failure = "worker_process_group_failed"
            } else {
                failure = "worker_launch_failed"
            }
            reply(
                peer: peer,
                request: request,
                fields: [
                    "outcome": "worker_spawn_failed",
                    "failure": failure,
                    "audio_content_retained": false,
                ]
            )
        }
    }

    private func stop(peerID: String, peer: xpc_connection_t, request: xpc_object_t) {
        guard activeLeaseID == peerID, let activeWorker = worker else {
            reply(
                peer: peer,
                request: request,
                fields: ["outcome": "not_active", "audio_content_retained": false]
            )
            return
        }
        let finalGeneration = generation
        let finalMode = activeMode
        activeLeaseID = nil
        activeWorker.stop { termination in
            let evidence = activeWorker.evidence()
            self.queue.async {
                guard self.worker === activeWorker,
                      self.generation == finalGeneration else { return }
                self.worker = nil
                var fields: [String: Any] = [
                    "outcome": termination.workerExited
                        && termination.processGroupEmpty
                        && termination.kind != "kill_unconfirmed"
                        ? "stopped"
                        : "stop_failed",
                    "generation": finalGeneration,
                    "synthetic_canary_count": evidence.canaryCount,
                    "worker_termination": termination.kind,
                    "stop_elapsed_ms": termination.elapsedMS,
                    "worker_exited": termination.workerExited,
                    "process_group_empty": termination.processGroupEmpty,
                    "exit_signal": termination.exitSignal,
                    "audio_content_retained": false,
                ]
                if finalMode.isSynthetic,
                   let first = evidence.firstSyntheticSequence,
                   let last = evidence.lastSyntheticSequence {
                    fields["synthetic_fixture"] = syntheticFixture
                    fields["synthetic_digest"] = syntheticFixtureDigest
                    fields["synthetic_first_sequence"] = first
                    fields["synthetic_last_sequence"] = last
                    fields["synthetic_complete"] =
                        first == 0
                            && last == syntheticFixtureChunks - 1
                            && evidence.canaryCount == syntheticFixtureChunks
                }
                self.reply(
                    peer: peer,
                    request: request,
                    fields: fields
                )
            }
        }
    }

    private func interruptActive() {
        guard let activeWorker = worker else {
            activeLeaseID = nil
            return
        }
        let interrupted = PendingRecovery(
            generation: generation,
            agentPID: UInt64(getpid()),
            agentBootNonce: instanceFingerprint,
            workerPID: activeWorker.pid,
            syntheticCanaryCount: canaryCount,
            firstCallbackMS: firstCallbackMS,
            workerTermination: "settling",
            workerExited: false,
            processGroupEmpty: false,
            exitSignal: 0,
            stopElapsedMS: 0,
            expiresAt: .now() + .milliseconds(Int(recoveryTTL * 1_000)),
            syntheticFixture: activeMode.isSynthetic ? syntheticFixture : nil,
            syntheticDigest: activeMode.isSynthetic ? syntheticFixtureDigest : nil,
            syntheticFirstSequence: firstSyntheticSequence,
            syntheticLastSequence: lastSyntheticSequence,
            claimID: nil,
            claimOwner: nil
        )
        activeLeaseID = nil
        pending = interrupted
        scheduleExpiry(generation: interrupted.generation, at: interrupted.expiresAt)
        activeWorker.stop { termination in
            let evidence = activeWorker.evidence()
            self.queue.async {
                guard self.worker === activeWorker,
                      self.pending?.generation == interrupted.generation else { return }
                self.worker = nil
                guard interrupted.expiresAt.uptimeNanoseconds > DispatchTime.now().uptimeNanoseconds else {
                    self.markExpired(generation: interrupted.generation)
                    self.pending = nil
                    return
                }
                self.pending = PendingRecovery(
                    generation: interrupted.generation,
                    agentPID: interrupted.agentPID,
                    agentBootNonce: interrupted.agentBootNonce,
                    workerPID: interrupted.workerPID,
                    syntheticCanaryCount: evidence.canaryCount,
                    firstCallbackMS: evidence.firstCallbackMS,
                    workerTermination: termination.kind,
                    workerExited: termination.workerExited,
                    processGroupEmpty: termination.processGroupEmpty,
                    exitSignal: termination.exitSignal,
                    stopElapsedMS: termination.elapsedMS,
                    expiresAt: interrupted.expiresAt,
                    syntheticFixture: interrupted.syntheticFixture,
                    syntheticDigest: interrupted.syntheticDigest,
                    syntheticFirstSequence: evidence.firstSyntheticSequence,
                    syntheticLastSequence: evidence.lastSyntheticSequence,
                    claimID: interrupted.claimID,
                    claimOwner: interrupted.claimOwner
                )
            }
        }
    }

    private func recover(
        peerID: String,
        peer: xpc_connection_t,
        request: xpc_object_t
    ) {
        expirePending()
        if let tombstone = acknowledged {
            reply(
                peer: peer,
                request: request,
                fields: [
                    "outcome": "already_acked",
                    "generation": tombstone.recovery.generation,
                    "recovered": false,
                    "exact_once": true,
                    "audio_content_retained": false,
                ]
            )
            return
        }
        if let expiredGeneration {
            reply(
                peer: peer,
                request: request,
                fields: [
                    "outcome": "expired",
                    "generation": expiredGeneration,
                    "recovered": false,
                    "audio_content_retained": false,
                ]
            )
            return
        }
        if let recovery = pending, recovery.workerTermination == "settling" {
            reply(
                peer: peer,
                request: request,
                fields: [
                    "outcome": "settling",
                    "recovered": false,
                    "audio_content_retained": false,
                ]
            )
            return
        }
        if let recovery = pending,
           (!recovery.workerExited || !recovery.processGroupEmpty) {
            reply(
                peer: peer,
                request: request,
                fields: [
                    "outcome": "isolation_failed",
                    "recovered": false,
                    "worker_exited": recovery.workerExited,
                    "process_group_empty": recovery.processGroupEmpty,
                    "audio_content_retained": false,
                ]
            )
            return
        }
        if let recovery = pending,
           recovery.syntheticFixture != nil,
           !(recovery.syntheticFirstSequence == 0
                && recovery.syntheticLastSequence == syntheticFixtureChunks - 1
                && recovery.syntheticCanaryCount == syntheticFixtureChunks) {
            reply(
                peer: peer,
                request: request,
                fields: [
                    "outcome": "synthetic_incomplete",
                    "recovered": false,
                    "audio_content_retained": false,
                ]
            )
            return
        }
        guard var recovery = pending, recovery.syntheticCanaryCount > 0 else {
            reply(
                peer: peer,
                request: request,
                fields: [
                    "outcome": "none",
                    "recovered": false,
                    "audio_content_retained": false,
                ]
            )
            return
        }
        if let owner = recovery.claimOwner, owner != peerID {
            reply(
                peer: peer,
                request: request,
                fields: [
                    "outcome": "claim_busy",
                    "recovered": false,
                    "audio_content_retained": false,
                ]
            )
            return
        }
        let claimID = recovery.claimID ?? UUID().uuidString
        if recovery.claimID == nil {
            recovery.claimID = claimID
            recovery.claimOwner = peerID
            pending = recovery
        }
        var fields = recoveryFields(recovery)
        fields["outcome"] = "recovery_offer"
        fields["claim_id"] = claimID
        fields["recovered"] = false
        fields["exact_once"] = false
        reply(peer: peer, request: request, fields: fields)
    }

    private func ack(
        peerID: String,
        claimID: String,
        peer: xpc_connection_t,
        request: xpc_object_t
    ) {
        expirePending()
        if let tombstone = acknowledged,
           tombstone.recovery.claimID == claimID,
           tombstone.recovery.claimOwner == peerID {
            var fields = recoveryFields(tombstone.recovery)
            fields["outcome"] = "recovery_acked"
            fields["claim_id"] = claimID
            fields["recovered"] = true
            fields["exact_once"] = true
            reply(peer: peer, request: request, fields: fields)
            return
        }
        guard !claimID.isEmpty,
              let recovery = pending,
              recovery.claimID == claimID,
              recovery.claimOwner == peerID,
              recovery.workerTermination != "settling" else {
            reply(
                peer: peer,
                request: request,
                fields: [
                    "outcome": "claim_rejected",
                    "recovered": false,
                    "exact_once": false,
                    "audio_content_retained": false,
                ]
            )
            return
        }
        pending = nil
        acknowledged = AcknowledgedTombstone(recovery: recovery)
        scheduleAcknowledgedExpiry(
            claimID: claimID,
            at: recovery.expiresAt
        )
        var fields = recoveryFields(recovery)
        fields["outcome"] = "recovery_acked"
        fields["claim_id"] = claimID
        fields["recovered"] = true
        fields["exact_once"] = true
        reply(peer: peer, request: request, fields: fields)
    }

    private func recoveryFields(_ recovery: PendingRecovery) -> [String: Any] {
        var fields: [String: Any] = [
            "generation": recovery.generation,
            "agent_pid": recovery.agentPID,
            "agent_instance": recovery.agentBootNonce,
            "worker_pid": recovery.workerPID,
            "synthetic_canary_count": recovery.syntheticCanaryCount,
            "first_callback_ms": recovery.firstCallbackMS,
            "worker_termination": recovery.workerTermination,
            "stop_elapsed_ms": recovery.stopElapsedMS,
            "recovery_ttl_ms": remainingMilliseconds(until: recovery.expiresAt),
            "agent_survived": true,
            "worker_exited": recovery.workerExited,
            "process_group_empty": recovery.processGroupEmpty,
            "exit_signal": recovery.exitSignal,
            "audio_content_retained": false,
        ]
        if let fixture = recovery.syntheticFixture,
           let digest = recovery.syntheticDigest,
           let first = recovery.syntheticFirstSequence,
           let last = recovery.syntheticLastSequence {
            fields["synthetic_fixture"] = fixture
            fields["synthetic_digest"] = digest
            fields["synthetic_first_sequence"] = first
            fields["synthetic_last_sequence"] = last
            fields["synthetic_complete"] =
                first == 0
                    && last == syntheticFixtureChunks - 1
                    && recovery.syntheticCanaryCount == syntheticFixtureChunks
        }
        return fields
    }

    private func status(peer: xpc_connection_t, request: xpc_object_t) {
        expirePending()
        let activeWorkerPID = worker?.pid ?? pending?.workerPID ?? 0
        let visibleCanaries = activeLeaseID == nil
            ? (pending?.syntheticCanaryCount ?? 0)
            : canaryCount
        reply(
            peer: peer,
            request: request,
            fields: [
                "outcome": activeLeaseID == nil ? (pending == nil ? "idle" : "pending") : "active",
                "agent_pid": UInt64(getpid()),
                "agent_instance": instanceFingerprint,
                "generation": generation,
                "worker_pid": activeWorkerPID,
                "synthetic_canary_count": visibleCanaries,
                "audio_content_retained": false,
            ]
        )
    }

    private func expirePending() {
        if let recovery = pending,
           recovery.expiresAt.uptimeNanoseconds <= DispatchTime.now().uptimeNanoseconds,
           worker == nil {
            markExpired(generation: recovery.generation)
            pending = nil
        }
        if let tombstone = acknowledged,
           tombstone.recovery.expiresAt.uptimeNanoseconds
            <= DispatchTime.now().uptimeNanoseconds {
            markExpired(generation: tombstone.recovery.generation)
            acknowledged = nil
        }
    }

    private func remainingMilliseconds(until deadline: DispatchTime) -> UInt64 {
        let now = DispatchTime.now().uptimeNanoseconds
        guard deadline.uptimeNanoseconds > now else { return 0 }
        return (deadline.uptimeNanoseconds - now) / 1_000_000
    }

    private func markExpired(generation: UInt64) {
        expiredGeneration = generation
    }

    private func scheduleExpiry(generation: UInt64, at deadline: DispatchTime) {
        queue.asyncAfter(deadline: deadline) {
            guard self.pending?.generation == generation,
                  self.worker == nil else {
                return
            }
            self.markExpired(generation: generation)
            self.pending = nil
        }
    }

    private func scheduleAcknowledgedExpiry(claimID: String, at deadline: DispatchTime) {
        queue.asyncAfter(deadline: deadline) {
            guard let tombstone = self.acknowledged,
                  tombstone.recovery.claimID == claimID else { return }
            self.acknowledged = nil
            self.markExpired(generation: tombstone.recovery.generation)
        }
    }

    private func reply(
        peer: xpc_connection_t,
        request: xpc_object_t,
        fields: [String: Any]
    ) {
        guard let response = xpc_dictionary_create_reply(request) else { return }
        setXPCUInt64(response, "schema_version", 1)
        for (key, value) in fields {
            switch value {
            case let text as String:
                setXPCString(response, key, text)
            case let number as UInt64:
                setXPCUInt64(response, key, number)
            case let number as Int:
                setXPCUInt64(response, key, UInt64(number))
            case let flag as Bool:
                setXPCBool(response, key, flag)
            default:
                continue
            }
        }
        xpc_connection_send_message(peer, response)
    }
}

private func applyPeerRequirement(_ connection: xpc_connection_t) -> Bool {
    peerRequirement.withCString { requirement in
        xpc_connection_set_peer_code_signing_requirement(connection, requirement) == 0
    }
}

private func makeClientConnection() -> xpc_connection_t? {
    let connection = xpc_connection_create_mach_service(machServiceName, nil, 0)
    guard applyPeerRequirement(connection) else {
        xpc_connection_cancel(connection)
        return nil
    }
    xpc_connection_set_event_handler(connection) { _ in }
    xpc_connection_activate(connection)
    return connection
}

private func sendRequest(
    connection: xpc_connection_t,
    command: String,
    timeout: TimeInterval = 8
) -> xpc_object_t? {
    let request = xpc_dictionary_create_empty()
    setXPCString(request, "command", command)
    let semaphore = DispatchSemaphore(value: 0)
    let lock = NSLock()
    var result: xpc_object_t?
    xpc_connection_send_message_with_reply(connection, request, nil) { reply in
        lock.lock()
        result = xpc_get_type(reply) == XPC_TYPE_DICTIONARY ? reply : nil
        lock.unlock()
        semaphore.signal()
    }
    guard semaphore.wait(timeout: .now() + timeout) == .success else { return nil }
    lock.lock()
    defer { lock.unlock() }
    return result
}

private func runClient(_ command: String) -> Int32 {
    guard let connection = makeClientConnection() else {
        jsonLine([
            "schema_version": 1,
            "outcome": "unsupported_os",
            "audio_content_retained": false,
        ])
        return 2
    }
    guard let reply = sendRequest(connection: connection, command: command) else {
        xpc_connection_cancel(connection)
        jsonLine([
            "schema_version": 1,
            "outcome": "xpc_timeout",
            "audio_content_retained": false,
        ])
        return 2
    }
    guard let response = dictionary(from: reply) else {
        xpc_connection_cancel(connection)
        return 2
    }
    jsonLine(response)
    xpc_connection_cancel(connection)
    let allowed: Set<String> = command == "status"
        ? ["idle", "active", "pending"]
        : []
    guard let outcome = response["outcome"] as? String,
          allowed.contains(outcome) else {
        return 2
    }
    return 0
}

private func replayEquivalent(_ lhs: [String: Any], _ rhs: [String: Any]) -> Bool {
    var first = lhs
    var second = rhs
    first.removeValue(forKey: "recovery_ttl_ms")
    second.removeValue(forKey: "recovery_ttl_ms")
    return NSDictionary(dictionary: first).isEqual(to: second)
}

private func runRecoveryClient(replayAcknowledgement: Bool) -> Int32 {
    guard let connection = makeClientConnection() else {
        jsonLine([
            "schema_version": 1,
            "outcome": "unsupported_os",
            "audio_content_retained": false,
        ])
        return 2
    }
    guard let reply = sendRequest(connection: connection, command: "recover") else {
        xpc_connection_cancel(connection)
        jsonLine([
            "schema_version": 1,
            "outcome": "xpc_timeout",
            "audio_content_retained": false,
        ])
        return 2
    }
    guard let offer = dictionary(from: reply) else {
        xpc_connection_cancel(connection)
        return 2
    }
    jsonLine(offer)
    if ["recovery_acked", "already_acked"].contains(
        offer["outcome"] as? String ?? ""
    ) {
        xpc_connection_cancel(connection)
        return 0
    }
    guard offer["outcome"] as? String == "recovery_offer" else {
        xpc_connection_cancel(connection)
        return ["none", "settling", "expired"].contains(
            offer["outcome"] as? String ?? ""
        ) ? 0 : 2
    }
    guard let claimID = offer["claim_id"] as? String,
          !claimID.isEmpty,
          readLine() == "ack:\(claimID)" else {
        xpc_connection_cancel(connection)
        return 2
    }
    var ackReply: xpc_object_t?
    for _ in 0..<2 {
        ackReply = sendRequest(connection: connection, command: "ack:\(claimID)")
        if ackReply != nil { break }
    }
    guard let ackReply else {
        xpc_connection_cancel(connection)
        return 2
    }
    guard let firstAck = dictionary(from: ackReply),
          firstAck["outcome"] as? String == "recovery_acked" else {
        xpc_connection_cancel(connection)
        return 2
    }
    let ack: [String: Any]
    if replayAcknowledgement {
        guard let replayReply = sendRequest(
            connection: connection,
            command: "ack:\(claimID)"
        ),
        let replay = dictionary(from: replayReply),
        replay["outcome"] as? String == "recovery_acked",
        replayEquivalent(firstAck, replay) else {
            xpc_connection_cancel(connection)
            return 2
        }
        ack = replay
    } else {
        ack = firstAck
    }
    jsonLine(ack)
    xpc_connection_cancel(connection)
    return ack["outcome"] as? String == "recovery_acked" ? 0 : 2
}

private func runLeaseClient(startCommand: String) -> Int32 {
    guard let connection = makeClientConnection() else {
        jsonLine([
            "schema_version": 1,
            "outcome": "unsupported_os",
            "audio_content_retained": false,
        ])
        return 2
    }
    guard let reply = sendRequest(connection: connection, command: startCommand) else {
        xpc_connection_cancel(connection)
        jsonLine([
            "schema_version": 1,
            "outcome": "xpc_timeout",
            "audio_content_retained": false,
        ])
        return 2
    }
    guard let ready = dictionary(from: reply) else {
        xpc_connection_cancel(connection)
        return 2
    }
    jsonLine(ready)
    guard ready["outcome"] as? String == "ready" else {
        xpc_connection_cancel(connection)
        return 2
    }
    while let line = readLine() {
        if line == "stop" {
            if let stopped = sendRequest(connection: connection, command: "stop"),
               let response = dictionary(from: stopped) {
                jsonLine(response)
            }
            break
        }
    }
    xpc_connection_cancel(connection)
    return 0
}

private func runServer() -> Never {
    let state = AgentState()
    let listener = xpc_connection_create_mach_service(
        machServiceName,
        nil,
        UInt64(XPC_CONNECTION_MACH_SERVICE_LISTENER)
    )
    xpc_connection_set_event_handler(listener) { event in
        guard xpc_get_type(event) == XPC_TYPE_CONNECTION else { return }
        let peer = event
        let peerID = UUID().uuidString
        guard applyPeerRequirement(peer) else {
            xpc_connection_cancel(peer)
            return
        }
        xpc_connection_set_event_handler(peer) { request in
            if xpc_get_type(request) == XPC_TYPE_DICTIONARY {
                state.handle(peerID: peerID, peer: peer, request: request)
            } else {
                state.disconnected(peerID: peerID)
            }
        }
        xpc_connection_activate(peer)
    }
    xpc_connection_activate(listener)
    dispatchMain()
}

@main
private struct CaptureAgentMain {
    static func main() {
        disableCoreDumps()
        let arguments = Array(CommandLine.arguments.dropFirst())
        if arguments.isEmpty {
            runServer()
        } else if arguments == ["lease"] {
            exit(runLeaseClient(startCommand: "start"))
        } else if arguments == ["lease-synthetic"] {
            exit(runLeaseClient(startCommand: "start_synthetic"))
        } else if arguments == ["lease-synthetic-fault"] {
            exit(runLeaseClient(startCommand: "start_synthetic_fault"))
        } else if arguments == ["recover"] {
            exit(runRecoveryClient(replayAcknowledgement: false))
        } else if arguments == ["recover-replay-ack"] {
            exit(runRecoveryClient(replayAcknowledgement: true))
        } else if arguments == ["status"] {
            exit(runClient("status"))
        } else {
            jsonLine([
                "schema_version": 1,
                "outcome": "invalid_arguments",
                "audio_content_retained": false,
            ])
            exit(64)
        }
    }
}
