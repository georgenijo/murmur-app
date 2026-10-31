// Hex a07f994 dot_scope.rs / spring.rs constants (MIT, Christopher Langton,
// 2026). Murmur's native implementation; see THIRD_PARTY_NOTICES.md.
import AppKit
import MetalKit
import CoreImage

private let tau = 2 * Double.pi
private func clamp(_ x: Double, _ low: Double = 0, _ high: Double = 1) -> Double { min(high, max(low, x)) }
private func follow(_ x: Double, _ target: Double, _ seconds: Double, _ dt: Double) -> Double {
    x + (target - x) * (1 - exp(-dt / seconds))
}

struct DotSpring {
    var x: Double
    var v = 0.0
    mutating func step(_ target: Double, _ bounce: Double, _ dt: Double) {
        let w = tau / 0.3, z = 1 - bounce, x0 = x - target, v0 = v
        let decay = z * w, e = exp(-decay * dt)
        if bounce > 0 {
            let wd = w * sqrt(1 - z * z), c = cos(wd * dt), s = sin(wd * dt) / wd
            x = target + e * (x0 * (c + decay * s) + v0 * s)
            v = e * (-x0 * w * w * s + v0 * (c - decay * s))
        } else {
            let c = v0 + w * x0
            x = target + (x0 + c * dt) * e
            v = (v0 - w * c * dt) * e
        }
    }
    mutating func jump(_ value: Double) { x = value; v = 0 }
}

// Fixed 240Hz state, independent of the display's refresh. All working arrays
// are allocated once. Text, audio samples and transcripts never reach this view.
struct DotMetrics: Equatable {
    var idleW = 30.0, recordingW = 112.0, processingW = 76.0, height = 26.0, radius = 11.0
}
final class NativeDotScope {
    let metrics: DotMetrics
    var width: DotSpring
    var scale = DotSpring(x: 0.9)
    var opacity = DotSpring(x: 0), blur = DotSpring(x: 3)
    var dots = [Float](repeating: 0, count: 387)
    var phosphor = [Float](repeating: 0, count: 387)
    var history = [Float](repeating: 0, count: 512)
    var stage = 0, head = 0
    var peak = -30.0, room = -55.0, envelope = 0.0, amplitude = 0.0, velocity = 0.0
    var remainder = 0.0, elapsed = 0.0, time = 0.0
    init(_ metrics: DotMetrics = DotMetrics()) { self.metrics = metrics; width = DotSpring(x: metrics.idleW) }
    func advance(_ dt: Double, _ next: Int, _ db: Double, _ reduced: Bool) {
        if next != stage { stage = next; elapsed = 0 }
        let h = 1.0 / 240
        remainder = min(remainder + max(dt.isFinite ? dt : 0, 0), h * 12)
        var steps = 0
        while remainder + 1e-10 >= h && steps < 12 {
            remainder -= h
            var level = 0.0
            if stage == 1 {
                let x = clamp(db.isFinite ? db : -120, -120, 0)
                room = follow(room, x, x < room ? 0.3 : 6, h)
                let lowest = max(room, -60) + 16
                peak = max(lowest, follow(peak, x > peak ? x : lowest, x > peak ? 0.04 : 3, h))
                let low = max(max(room, -60) + 5, peak - 24)
                level = clamp((x - low) / max(peak - low, 1))
            }
            envelope = follow(envelope, level, level > envelope ? 0.022 : 0.15, h)
            let omega = tau * 6.5
            velocity += (-omega * omega * (amplitude - envelope) - 2 * 0.52 * omega * velocity) * h
            amplitude += velocity * h
            history[head] = Float(clamp(amplitude)); head = (head + 1) % 512
            let target = stage == 1 ? metrics.recordingW : stage == 2 || stage == 3 ? metrics.processingW : metrics.idleW
            let shown = stage == 0 ? 0.0 : stage == 3 ? clamp(1 - elapsed / 0.4) : 1.0
            if reduced { width.jump(target); scale.jump(1); blur.jump(0) }
            else {
                width.step(target, 0.15, h); scale.step(stage == 0 ? 0.9 : 1, 0.15, h)
                blur.step(stage == 0 ? 3 : 0, 0, h)
            }
            opacity.step(shown, 0, h); elapsed += h; time += h; steps += 1
        }
    }
    func visibility(_ distance: Double, _ reduced: Bool) -> Double {
        stage == 3 && !reduced ? clamp((distance - elapsed * (max(width.x, 26) / 2 + 12) / 0.3) / 6) : 1
    }
    func sample(_ dt: Double, _ style: Int, _ reduced: Bool) {
        let keep = exp(-min(max(dt, 0), 0.05) / 0.6), half = max(width.x, 26) / 2
        for c in 0..<43 {
            let x = Double(c - 21) * 2.6, distance = abs(x)
            let delay = distance / 150 * 240, past = head - 1 - Int(floor(delay))
            let a = (past + 1024) % 512, b = (a + 511) % 512
            let amplitude = reduced ? 0.35 : Double(history[a]) + (Double(history[b]) - Double(history[a])) * delay.truncatingRemainder(dividingBy: 1)
            let edge = clamp((half - distance) / 14), falloff = 1 - 0.8 * clamp(distance / half)
            let phase = tau * 1.6 * (x / 112 + 0.5) - (reduced ? 0 : time * tau * 0.6)
            let y = sin(phase) * (0.7 + 3.1 * amplitude * falloff) * edge
            var work = 0.0
            if stage == 2 || stage == 3 {
                if reduced { work = 0.4 }
                else if style == 0 { work = exp(-pow((x - sin(time * tau / 1.1) * half) / 7, 2)) }
                else if style == 1 { work = pow(0.5 + 0.5 * cos(x / 8 - time * tau / 1.1), 4) }
                else { work = exp(-pow((distance - (elapsed * 46).truncatingRemainder(dividingBy: half + 12)) / 7, 2)) }
            }
            let wipe = visibility(distance, reduced)
            for r in 0...4 {
                let n = r * 43 + c, opposite = (8 - r) * 43 + c
                let trace = exp(-pow((Double(4 - r) - abs(y)) / 0.68, 2))
                let power = edge * falloff * trace * (0.2 + 0.8 * (stage == 1 ? amplitude : work))
                phosphor[n] = Float(reduced ? power : max(power, Double(phosphor[n]) * keep) * wipe)
                dots[n] = phosphor[n]; dots[opposite] = phosphor[n]; phosphor[opposite] = phosphor[n]
            }
        }
    }
    func write(_ points: UnsafeMutablePointer<SIMD4<Float>>, _ reduced: Bool) {
        for c in 0..<43 {
            let x = Double(c - 21) * 2.6, distance = abs(x)
            let mask = clamp((width.x / 2 - distance - 1.4) / 3.2), visible = visibility(distance, reduced)
            for r in 0..<9 {
                let n = r * 43 + c, y = Double(r - 4) * 2.6, light = Double(dots[n])
                let floor = 0.07 * (1 - 0.7 * abs(y) / 13)
                let alpha = (floor + (0.96 - floor) * pow(light, 0.9)) * mask * clamp(opacity.x) * visible
                points[n] = SIMD4(Float(x), Float(y), Float((0.25 + 0.17 * pow(light, 0.6)) * 2.6), Float(round(alpha * 23) / 23))
            }
        }
    }
}

private let shader = """
#include <metal_stdlib>
using namespace metal;
struct Params { float2 size; float2 halfClip; float scaleRatio; float corner; float2 pad; float4 color; };
struct Dot { float4 position [[position]]; float pointSize [[point_size]]; float size; float radius; float alpha; };
vertex Dot dotsVertex(uint id [[vertex_id]], constant float4 *dots [[buffer(0)]], constant Params &p [[buffer(1)]]) {
    Dot o; float4 d = dots[id]; float2 xy = d.xy * p.scaleRatio;
    o.position = float4(xy * float2(2, -2) / p.size, 0, 1);
    o.radius = d.z * p.scaleRatio; o.size = ceil(2 * o.radius) + 2; o.pointSize = o.size; o.alpha = d.w; return o;
}
fragment float4 dotsFragment(Dot d [[stage_in]], float2 point [[point_coord]], constant Params &p [[buffer(1)]]) {
    float circle = saturate(d.radius + 0.5 - length((point - 0.5) * d.size));
    float2 q = abs((d.position.xy - p.size * 0.5) / p.scaleRatio) - p.halfClip + p.corner;
    float dist = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - p.corner;
    float a = d.alpha * circle * saturate(0.5 - dist * p.scaleRatio);
    return float4(p.color.rgb * a, a);
}
"""

private struct Params {
    var size: SIMD2<Float>, halfClip: SIMD2<Float>
    var scaleRatio: Float, corner: Float
    var pad = SIMD2<Float>(repeating: 0)
    var color: SIMD4<Float>
}
private let levelLock = NSLock()
private var latestLevel = 0.0, latestLevelAt = -Double.infinity
@_cdecl("murmur_overlay_dot_level")
public func overlayDotLevel(_ value: Double) {
    levelLock.lock(); latestLevel = value.isFinite ? max(0, value) : 0
    latestLevelAt = CACurrentMediaTime(); levelLock.unlock()
}

private final class DotMetalView: MTKView {
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
    override var acceptsFirstResponder: Bool { false }
}
// Explicit local measurement mode only. Actual drawable presentation times,
// not timer/RAF counts, establish native refresh pacing. Bounded and text-free.
private final class DotPresentationStats {
    let lock = NSLock()
    var times = [Double]()
    init() { times.reserveCapacity(1800) }
    func presented(_ time: Double) {
        guard time > 0 else { return }
        lock.lock(); if times.count < 1800 { times.append(time) }; lock.unlock()
    }
    func report() {
        lock.lock(); let values = times; lock.unlock()
        guard let first = values.first else { return }
        let settled = Array(Set(values)).sorted().filter { $0 > first + 2 }
        guard let start = settled.first, let end = settled.last, end > start else { return }
        let gaps = zip(settled.dropFirst(), settled).map { $0 - $1 }
        let report: [String: Any] = ["presentedFrames": settled.count, "seconds": end - start,
            "fps": Double(settled.count - 1) / (end - start), "maxGapSeconds": gaps.max() ?? 0,
            "method": "MTLDrawable.presentedTime after two-second warmup"]
        if let data = try? JSONSerialization.data(withJSONObject: report, options: .sortedKeys), let json = String(data: data, encoding: .utf8) {
            fputs("MURMUR_DOT_PRESENTATION \(json)\n", stderr)
        }
    }
}
private final class DotRenderer: NSObject, MTKViewDelegate {
    let view: DotMetalView, queue: MTLCommandQueue, pipeline: MTLRenderPipelineState
    let buffers: [MTLBuffer]
    let inFlight = DispatchSemaphore(value: 3)
    var bufferIndex = 0
    let scope: NativeDotScope
    weak var window: NSWindow?
    var stage = 0, style = 1, reduced = false, visible = true, top = 0.0, last = 0.0
    var lastBlur = -1
    var presentation: DotPresentationStats?
    let measurePresentation = ProcessInfo.processInfo.environment["MURMUR_OVERLAY_METRICS"] == "1"
    var observers = [NSObjectProtocol]()
    init?(_ window: NSWindow, _ metrics: DotMetrics) {
        guard let device = MTLCreateSystemDefaultDevice(), let queue = device.makeCommandQueue(),
              let library = try? device.makeLibrary(source: shader, options: nil) else { return nil }
        let buffers = (0..<3).compactMap { _ in device.makeBuffer(length: 387 * MemoryLayout<SIMD4<Float>>.stride, options: .storageModeShared) }
        guard buffers.count == 3 else { return nil }
        let descriptor = MTLRenderPipelineDescriptor()
        descriptor.vertexFunction = library.makeFunction(name: "dotsVertex")
        descriptor.fragmentFunction = library.makeFunction(name: "dotsFragment")
        let color = descriptor.colorAttachments[0]!
        color.pixelFormat = .bgra8Unorm; color.isBlendingEnabled = true
        color.sourceRGBBlendFactor = .one; color.destinationRGBBlendFactor = .oneMinusSourceAlpha
        color.sourceAlphaBlendFactor = .one; color.destinationAlphaBlendFactor = .oneMinusSourceAlpha
        guard let pipeline = try? device.makeRenderPipelineState(descriptor: descriptor), let parent = window.contentView else { return nil }
        self.window = window; self.queue = queue; self.pipeline = pipeline; self.buffers = buffers
        self.scope = NativeDotScope(metrics)
        view = DotMetalView(frame: NSRect(x: 0, y: 0, width: metrics.recordingW, height: metrics.height), device: device)
        super.init()
        view.colorPixelFormat = .bgra8Unorm; view.clearColor = MTLClearColorMake(0, 0, 0, 0)
        view.depthStencilPixelFormat = .invalid; view.isPaused = true
        view.layer?.isOpaque = false; view.setAccessibilityElement(false); view.delegate = self
        parent.addSubview(view, positioned: .above, relativeTo: nil)
        parent.postsFrameChangedNotifications = true
        observers.append(NotificationCenter.default.addObserver(forName: NSView.frameDidChangeNotification, object: parent, queue: .main) { [weak self] _ in self?.anchor() })
        observers.append(NotificationCenter.default.addObserver(forName: NSWindow.didChangeScreenNotification, object: window, queue: .main) { [weak self] _ in self?.anchor() })
        observers.append(NotificationCenter.default.addObserver(forName: NSWindow.didChangeOcclusionStateNotification, object: window, queue: .main) { [weak self] _ in self?.resume() })
        anchor()
    }
    deinit { for observer in observers { NotificationCenter.default.removeObserver(observer) } }
    func stop() { view.isPaused = true; view.delegate = nil; view.removeFromSuperview() }
    func anchor() {
        guard let parent = window?.contentView else { return }
        let metrics = scope.metrics
        view.frame = NSRect(x: (parent.bounds.width - metrics.recordingW) / 2,
            y: parent.isFlipped ? top : parent.bounds.height - top - metrics.height, width: metrics.recordingW, height: metrics.height)
        view.preferredFramesPerSecond = window?.screen?.maximumFramesPerSecond ?? 60
        resume()
    }
    func resume() {
        last = 0
        let shown = visible && window?.isVisible == true
        view.isHidden = !shown || stage == 0
        view.isPaused = !shown || stage == 0
        if shown && stage != 0 { view.isPaused = false }
    }
    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {}
    func draw(in view: MTKView) {
        guard visible, window?.isVisible == true else { view.isPaused = true; return }
        guard inFlight.wait(timeout: .now()) == .success else { return }
        var committed = false
        defer { if !committed { inFlight.signal() } }
        let buffer = buffers[bufferIndex]
        let now = CACurrentMediaTime(), dt = last == 0 ? 1 / 60 : min(now - last, 0.05)
        last = now
        levelLock.lock(); let rms = latestLevel, at = latestLevelAt; levelLock.unlock()
        let db = now - at < 0.15 ? 20 * log10(max(rms, 1e-6)) : -120
        scope.advance(dt, stage, db, reduced); scope.sample(dt, style, reduced)
        scope.write(buffer.contents().assumingMemoryBound(to: SIMD4<Float>.self), reduced)
        let blur = Int(round(clamp(scope.blur.x, 0, 3) * 10))
        if blur != lastBlur {
            lastBlur = blur
            view.layer?.filters = blur == 0 ? nil : CIFilter(name: "CIGaussianBlur", parameters: [kCIInputRadiusKey: Double(blur) / 10]).map { [$0] }
        }
        guard let pass = view.currentRenderPassDescriptor, let drawable = view.currentDrawable,
              let command = queue.makeCommandBuffer(), let encoder = command.makeRenderCommandEncoder(descriptor: pass) else { return }
        var params = Params(size: SIMD2(Float(view.drawableSize.width), Float(view.drawableSize.height)),
            halfClip: SIMD2(Float(scope.width.x / 2), Float(scope.metrics.height / 2)), scaleRatio: Float(scope.scale.x * view.drawableSize.width / scope.metrics.recordingW), corner: Float(scope.metrics.radius),
            color: style == 0 ? SIMD4(200 / 255, 220 / 255, 248 / 255, 1) : SIMD4(222 / 255, 215 / 255, 201 / 255, 1))
        encoder.setRenderPipelineState(pipeline); encoder.setVertexBuffer(buffer, offset: 0, index: 0)
        encoder.setVertexBytes(&params, length: MemoryLayout<Params>.stride, index: 1)
        encoder.setFragmentBytes(&params, length: MemoryLayout<Params>.stride, index: 1)
        encoder.drawPrimitives(type: .point, vertexStart: 0, vertexCount: 387)
        encoder.endEncoding(); command.present(drawable)
        if let presentation = presentation { drawable.addPresentedHandler { presentation.presented($0.presentedTime) } }
        command.addCompletedHandler { [inFlight = self.inFlight] _ in inFlight.signal() }
        command.commit(); committed = true; bufferIndex = (bufferIndex + 1) % buffers.count
        if reduced && scope.opacity.x > 0.998 || stage == 3 && scope.opacity.x < 0.002 { view.isPaused = true }
    }
}
private var renderer: DotRenderer?

@_cdecl("murmur_overlay_dot_state")
public func overlayDotState(_ pointer: UnsafeMutableRawPointer, _ stage: Int32, _ style: Int32, _ reduced: Bool, _ visible: Bool, _ top: Double,
    _ idleW: Double, _ recordingW: Double, _ processingW: Double, _ height: Double, _ radius: Double) -> Bool {
    precondition(Thread.isMainThread)
    let window = Unmanaged<NSWindow>.fromOpaque(pointer).takeUnretainedValue()
    let metrics = DotMetrics(idleW: idleW, recordingW: recordingW, processingW: processingW, height: height, radius: radius)
    if renderer?.window !== window || renderer?.scope.metrics != metrics {
        renderer?.stop(); renderer = DotRenderer(window, metrics)
    }
    guard let current = renderer else { return false }
    if current.stage == 1 && stage != 1 { current.presentation?.report(); current.presentation = nil }
    if current.stage != 1 && stage == 1 && current.measurePresentation { current.presentation = DotPresentationStats() }
    current.stage = Int(stage); current.style = Int(style); current.reduced = reduced
    current.visible = visible; current.top = top; current.anchor()
    return true
}
