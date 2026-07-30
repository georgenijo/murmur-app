import Foundation
import Metal

private let warmupIterations = 20
private let measuredIterations = 300
private let elementCount = 256 * 1024
private let allocationProbeBytes = 64 * 1024 * 1024

private enum ProbeError: Error, CustomStringConvertible {
    case metalUnavailable
    case commandQueueUnavailable
    case libraryUnavailable(String)
    case functionUnavailable
    case pipelineUnavailable(String)
    case bufferUnavailable
    case commandBufferUnavailable
    case encoderUnavailable
    case commandBufferFailed(String)
    case counterBufferUnavailable(String)
    case counterDataUnavailable

    var description: String {
        switch self {
        case .metalUnavailable:
            return "Metal is unavailable"
        case .commandQueueUnavailable:
            return "Metal command queue creation failed"
        case let .libraryUnavailable(message):
            return "Metal library creation failed: \(message)"
        case .functionUnavailable:
            return "Probe kernel lookup failed"
        case let .pipelineUnavailable(message):
            return "Compute pipeline creation failed: \(message)"
        case .bufferUnavailable:
            return "Metal buffer allocation failed"
        case .commandBufferUnavailable:
            return "Metal command buffer creation failed"
        case .encoderUnavailable:
            return "Metal compute encoder creation failed"
        case let .commandBufferFailed(message):
            return "Metal command buffer failed: \(message)"
        case let .counterBufferUnavailable(message):
            return "Metal counter sample buffer creation failed: \(message)"
        case .counterDataUnavailable:
            return "Metal timestamp counter results were unavailable"
        }
    }
}

private enum InstrumentationMode: String, CaseIterable {
    case baseline
    case commandBufferTimestamps = "command_buffer_timestamps"
    case counterSampleBuffer = "counter_sample_buffer"
}

private struct RunSample {
    let wallMilliseconds: Double
        let gpuMilliseconds: Double?
    let counterDeltaTicks: UInt64?
}

private struct Distribution: Codable {
    let median: Double
    let p95: Double
    let minimum: Double
    let maximum: Double
}

private struct ModeResult: Codable {
    let exactMetricName: String
    let wallMilliseconds: Distribution
    let medianWallDeltaMicrosecondsVersusBaseline: Double
    let medianWallOverheadPercentVersusBaseline: Double
    let gpuElapsedMilliseconds: Distribution?
    let timestampCounterDeltaTicks: Distribution?
}

private struct SamplingPointSupport: Codable {
    let name: String
    let supported: Bool
}

private struct CounterSetDescription: Codable {
    let name: String
    let counters: [String]
}

private struct AllocationResult: Codable {
    let exactMetricName: String
    let requestedBytes: UInt64
    let beforeBytes: UInt64
    let afterAllocationBytes: UInt64
    let immediateAfterReleaseBytes: UInt64
    let afterReleaseAndDrainBytes: UInt64
}

private struct ProbeReport: Codable {
    let probeBoundary: String
    let deviceName: String
    let operatingSystem: String
    let architecture: String
    let unifiedMemory: Bool
    let warmupIterations: Int
    let measuredIterationsPerMode: Int
    let elementsPerDispatch: Int
    let counterSets: [CounterSetDescription]
    let samplingPoints: [SamplingPointSupport]
    let results: [String: ModeResult]
    let allocation: AllocationResult
}

private final class MetalProbe {
    private let device: MTLDevice
    private let queue: MTLCommandQueue
    private let pipeline: MTLComputePipelineState
    private let workBuffer: MTLBuffer
    private let timestampCounterBuffer: MTLCounterSampleBuffer?

    init() throws {
        guard let device = MTLCreateSystemDefaultDevice() else {
            throw ProbeError.metalUnavailable
        }
        guard let queue = device.makeCommandQueue() else {
            throw ProbeError.commandQueueUnavailable
        }

        let source = """
        #include <metal_stdlib>
        using namespace metal;

        kernel void probe_add_one(
            device uint *values [[buffer(0)]],
            uint index [[thread_position_in_grid]]
        ) {
            values[index] += 1;
        }
        """
        let library: MTLLibrary
        do {
            library = try device.makeLibrary(source: source, options: nil)
        } catch {
            throw ProbeError.libraryUnavailable(String(describing: error))
        }
        guard let function = library.makeFunction(name: "probe_add_one") else {
            throw ProbeError.functionUnavailable
        }
        do {
            pipeline = try device.makeComputePipelineState(function: function)
        } catch {
            throw ProbeError.pipelineUnavailable(String(describing: error))
        }
        guard let workBuffer = device.makeBuffer(
            length: elementCount * MemoryLayout<UInt32>.stride,
            options: .storageModeShared
        ) else {
            throw ProbeError.bufferUnavailable
        }

        self.device = device
        self.queue = queue
        self.workBuffer = workBuffer
        self.timestampCounterBuffer = try Self.makeTimestampCounterBuffer(device: device)
    }

    var metalDevice: MTLDevice {
        device
    }

    var counterBufferAvailable: Bool {
        timestampCounterBuffer != nil
    }

    private static func makeTimestampCounterBuffer(
        device: MTLDevice
    ) throws -> MTLCounterSampleBuffer? {
        guard device.supportsCounterSampling(.atStageBoundary) else {
            return nil
        }
        guard let timestampSet = device.counterSets?.first(where: {
            $0.name == MTLCommonCounterSet.timestamp.rawValue
        }) else {
            return nil
        }

        let descriptor = MTLCounterSampleBufferDescriptor()
        descriptor.counterSet = timestampSet
        descriptor.label = "Issue 354 timestamp probe"
        descriptor.storageMode = .shared
        descriptor.sampleCount = 2
        do {
            return try device.makeCounterSampleBuffer(descriptor: descriptor)
        } catch {
            throw ProbeError.counterBufferUnavailable(String(describing: error))
        }
    }

    func run(mode: InstrumentationMode) throws -> RunSample {
        let wallStarted = DispatchTime.now().uptimeNanoseconds
        guard let commandBuffer = queue.makeCommandBuffer() else {
            throw ProbeError.commandBufferUnavailable
        }

        let encoder: MTLComputeCommandEncoder?
        if mode == .counterSampleBuffer, let timestampCounterBuffer {
            let descriptor = MTLComputePassDescriptor()
            guard let attachment = descriptor.sampleBufferAttachments[0] else {
                throw ProbeError.encoderUnavailable
            }
            attachment.sampleBuffer = timestampCounterBuffer
            attachment.startOfEncoderSampleIndex = 0
            attachment.endOfEncoderSampleIndex = 1
            encoder = commandBuffer.makeComputeCommandEncoder(descriptor: descriptor)
        } else {
            encoder = commandBuffer.makeComputeCommandEncoder()
        }
        guard let encoder else {
            throw ProbeError.encoderUnavailable
        }

        encoder.setComputePipelineState(pipeline)
        encoder.setBuffer(workBuffer, offset: 0, index: 0)
        let threadsPerGrid = MTLSize(width: elementCount, height: 1, depth: 1)
        let width = min(pipeline.maxTotalThreadsPerThreadgroup, 256)
        let threadsPerThreadgroup = MTLSize(width: width, height: 1, depth: 1)
        encoder.dispatchThreads(threadsPerGrid, threadsPerThreadgroup: threadsPerThreadgroup)
        encoder.endEncoding()

        commandBuffer.commit()
        commandBuffer.waitUntilCompleted()

        guard commandBuffer.status == .completed else {
            throw ProbeError.commandBufferFailed(
                commandBuffer.error.map(String.init(describing:)) ?? "\(commandBuffer.status.rawValue)"
            )
        }

    let gpuMilliseconds: Double?
        if mode == .commandBufferTimestamps {
            gpuMilliseconds = (commandBuffer.gpuEndTime - commandBuffer.gpuStartTime) * 1_000
        } else {
            gpuMilliseconds = nil
        }

        let counterDeltaTicks: UInt64?
        if mode == .counterSampleBuffer, let timestampCounterBuffer {
            guard let data = try timestampCounterBuffer.resolveCounterRange(0..<2) else {
                throw ProbeError.counterDataUnavailable
            }
            let requiredBytes = 2 * MemoryLayout<MTLCounterResultTimestamp>.stride
            guard data.count >= requiredBytes else {
                throw ProbeError.counterDataUnavailable
            }
            counterDeltaTicks = data.withUnsafeBytes { rawBuffer in
                let samples = rawBuffer.bindMemory(to: MTLCounterResultTimestamp.self)
                let start = samples[0].timestamp
                let end = samples[1].timestamp
                guard start != MTLCounterErrorValue,
                      end != MTLCounterErrorValue,
                      end >= start else {
                    return nil
                }
                return end - start
            }
            if counterDeltaTicks == nil {
                throw ProbeError.counterDataUnavailable
            }
        } else {
            counterDeltaTicks = nil
        }

        let wallEnded = DispatchTime.now().uptimeNanoseconds
        let wallMilliseconds = Double(wallEnded - wallStarted) / 1_000_000
        return RunSample(
            wallMilliseconds: wallMilliseconds,
            gpuMilliseconds: gpuMilliseconds,
            counterDeltaTicks: counterDeltaTicks
        )
    }

    func allocationProbe() throws -> AllocationResult {
        let before = UInt64(device.currentAllocatedSize)
        var allocation: MTLBuffer? = device.makeBuffer(
            length: allocationProbeBytes,
            options: .storageModeShared
        )
        guard allocation != nil else {
            throw ProbeError.bufferUnavailable
        }
        let afterAllocation = UInt64(device.currentAllocatedSize)
        allocation = nil
        let immediateAfterRelease = UInt64(device.currentAllocatedSize)
        guard let drain = queue.makeCommandBuffer() else {
            throw ProbeError.commandBufferUnavailable
        }
        drain.commit()
        drain.waitUntilCompleted()
        guard drain.status == .completed else {
            throw ProbeError.commandBufferFailed(
                drain.error.map(String.init(describing:)) ?? "\(drain.status.rawValue)"
            )
        }
        autoreleasepool {}
        let afterReleaseAndDrain = UInt64(device.currentAllocatedSize)

        return AllocationResult(
            exactMetricName: "Metal resource allocation (bytes)",
            requestedBytes: UInt64(allocationProbeBytes),
            beforeBytes: before,
            afterAllocationBytes: afterAllocation,
            immediateAfterReleaseBytes: immediateAfterRelease,
            afterReleaseAndDrainBytes: afterReleaseAndDrain
        )
    }
}

private func distribution(_ values: [Double]) -> Distribution {
    let sorted = values.sorted()
    let medianIndex = sorted.count / 2
    let median: Double
    if sorted.count.isMultiple(of: 2) {
        median = (sorted[medianIndex - 1] + sorted[medianIndex]) / 2
    } else {
        median = sorted[medianIndex]
    }
    let p95Index = min(sorted.count - 1, Int(ceil(Double(sorted.count) * 0.95)) - 1)
    return Distribution(
        median: median,
        p95: sorted[p95Index],
        minimum: sorted[0],
        maximum: sorted[sorted.count - 1]
    )
}

private func architectureName() -> String {
    #if arch(arm64)
    return "arm64"
    #elseif arch(x86_64)
    return "x86_64"
    #else
    return "unknown"
    #endif
}

private func runProbe() throws -> ProbeReport {
    let probe = try MetalProbe()

    for _ in 0..<warmupIterations {
        _ = try probe.run(mode: .baseline)
    }

    var samples = Dictionary(
        uniqueKeysWithValues: InstrumentationMode.allCases.map { ($0, [RunSample]()) }
    )
    let measuredModes = probe.counterBufferAvailable
        ? InstrumentationMode.allCases
        : [.baseline, .commandBufferTimestamps]

    for iteration in 0..<measuredIterations {
        let offset = iteration % measuredModes.count
        let orderedModes = Array(measuredModes[offset...] + measuredModes[..<offset])
        for mode in orderedModes {
            samples[mode, default: []].append(try probe.run(mode: mode))
        }
    }

    let baselineDistribution = distribution(
        samples[.baseline, default: []].map(\.wallMilliseconds)
    )
    var results: [String: ModeResult] = [:]
    for mode in measuredModes {
        let modeSamples = samples[mode, default: []]
        let wall = distribution(modeSamples.map(\.wallMilliseconds))
        let medianDeltaMicroseconds = (wall.median - baselineDistribution.median) * 1_000
        let overheadPercent = baselineDistribution.median > 0
            ? (wall.median - baselineDistribution.median) / baselineDistribution.median * 100
            : 0
        let gpuValues = modeSamples.compactMap(\.gpuMilliseconds)
        let counterValues = modeSamples.compactMap(\.counterDeltaTicks).map { Double($0) }
        let exactMetricName: String
        switch mode {
        case .baseline:
            exactMetricName = "Probe command-buffer wall duration (ms)"
        case .commandBufferTimestamps:
            exactMetricName = "Metal command-buffer GPU elapsed time (ms)"
        case .counterSampleBuffer:
            exactMetricName = "Metal timestamp counter delta (ticks)"
        }
        results[mode.rawValue] = ModeResult(
            exactMetricName: exactMetricName,
            wallMilliseconds: wall,
            medianWallDeltaMicrosecondsVersusBaseline: medianDeltaMicroseconds,
            medianWallOverheadPercentVersusBaseline: overheadPercent,
            gpuElapsedMilliseconds: gpuValues.isEmpty ? nil : distribution(gpuValues),
            timestampCounterDeltaTicks: counterValues.isEmpty ? nil : distribution(counterValues)
        )
    }

    let device = probe.metalDevice
    let counterSets = (device.counterSets ?? []).map {
        CounterSetDescription(name: $0.name, counters: $0.counters.map(\.name))
    }
    let samplingPoints: [(String, MTLCounterSamplingPoint)] = [
        ("stage_boundary", .atStageBoundary),
        ("draw_boundary", .atDrawBoundary),
        ("dispatch_boundary", .atDispatchBoundary),
        ("tile_dispatch_boundary", .atTileDispatchBoundary),
        ("blit_boundary", .atBlitBoundary),
    ]

    return ProbeReport(
        probeBoundary: "Standalone public Metal API probe; not Murmur runtime instrumentation",
        deviceName: device.name,
        operatingSystem: ProcessInfo.processInfo.operatingSystemVersionString,
        architecture: architectureName(),
        unifiedMemory: device.hasUnifiedMemory,
        warmupIterations: warmupIterations,
        measuredIterationsPerMode: measuredIterations,
        elementsPerDispatch: elementCount,
        counterSets: counterSets,
        samplingPoints: samplingPoints.map {
            SamplingPointSupport(name: $0.0, supported: device.supportsCounterSampling($0.1))
        },
        results: results,
        allocation: try probe.allocationProbe()
    )
}

do {
    let report = try runProbe()
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
    let data = try encoder.encode(report)
    guard let output = String(data: data, encoding: .utf8) else {
        throw ProbeError.counterDataUnavailable
    }
    print(output)
} catch {
    FileHandle.standardError.write(Data("metal_metrics_probe: \(error)\n".utf8))
    exit(1)
}
