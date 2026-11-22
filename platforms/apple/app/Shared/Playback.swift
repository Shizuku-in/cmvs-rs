import CmvsAppleC
import QuartzCore
import SwiftUI

#if os(iOS)
    import AVFAudio
#endif

enum FastForwardKey: UInt32 {
    case shift = 23
    case control = 24
}

@MainActor @Observable final class Playback: NSObject, Identifiable {
    let game: Game
    let font: FontFace
    private(set) var dialogFont: FontFace?
    let fonts: [FontFace]
    let engine = Engine()
    let accessibility = GameAccessibility()
    let cpu = CALayer()
    private(set) var canvasFrame = CGRect.zero
    @ObservationIgnored private var viewportFrame = CGRect.zero
    @ObservationIgnored private var scaledCpuCanvas = false
    private var clock: CADisplayLink?
    private var previous: CFTimeInterval?
    private var frames = 0
    @ObservationIgnored private var measuringPerformance = false
    @ObservationIgnored private var sampleStart: (time: CFTimeInterval, frames: Int)?
    private(set) var framesPerSecond: Double?
    private(set) var debugRecording: DebugRecording?
    private(set) var debugReport: DebugReport?
    private var active = true
    private(set) var interrupted = false
    private var interruptionOngoing = false
    private var routePaused = false
    var audioError: String?
    private var focused = true
    private var closed = false
    private(set) var ready = false
    var canCapture: Bool { ready && frames > 0 && error == nil && dialog == nil }
    var captureError: String?
    var screenshotName: String?
    var didClose: (() -> Void)?
    var clearKeys: (() -> Void)?
    var warp: ((CGPoint) -> Void)?
    var title: String
    private(set) var error: String?
    var dialog: [String: Any]?
    var draft = ""
    var controls = false
    private(set) var lockedFastForward = Set<FastForwardKey>()
    @ObservationIgnored private var keyboardFastForward = Set<FastForwardKey>()
    var fullscreen: ((Bool) -> Void)?
    var windowState: (() -> (windowed: Bool, resizable: Bool))?
    var resizeWindow: ((CGSize) -> Void)?
    var cursorVisible: ((Bool) -> Void)?
    var cursor: (([String: Any]?) -> Void)?
    var closeError: ((String) -> Void)?
    var quit: (() -> Void)?
    var encoding: String
    private var logicalSize: [Int]
    private var viewSize = CGSize.zero
    private var scale: CGFloat = 1
    private var smokeFrames = Int(ProcessInfo.processInfo.environment["CMVS_SMOKE_FRAMES"] ?? "") ?? 0
    init(_ game: Game, _ font: FontFace, _ fonts: [FontFace]) {
        self.game = game
        self.font = font
        self.fonts = fonts
        title = game.name
        encoding = game.info.encoding
        logicalSize = game.info.size
    }
    func start(_ layer: CAMetalLayer, size: CGSize, scale: CGFloat, link: CADisplayLink) {
        guard clock == nil, !closed else {
            link.invalidate()
            return
        }
        clock = link
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 30, maximum: 60, preferred: 60)
        layer.addSublayer(cpu)
        layer.masksToBounds = true
        cpu.contentsGravity = .resize
        resize(size, scale)
        #if os(iOS)
            do {
                if game.options.native_audio {
                    let audio = AVAudioSession.sharedInstance()
                    try audio.setCategory(.playback)
                    try audio.setActive(true)
                    NotificationCenter.default.addObserver(
                        self, selector: #selector(audioInterrupted), name: AVAudioSession.interruptionNotification,
                        object: audio)
                    NotificationCenter.default.addObserver(
                        self, selector: #selector(routeChanged), name: AVAudioSession.routeChangeNotification,
                        object: audio)
                }
            } catch {
                fail(error)
                return
            }
        #endif
        let width = UInt32(size.width * scale)
        let height = UInt32(size.height * scale)
        let engine = engine
        let game = game
        let font = font
        let fonts = fonts
        engine.queue.async {
            let result = Result { try engine.open(game, font: font, fonts: fonts, layer: layer, width: width, height: height) }
            DispatchQueue.main.async {
                guard !self.closed else { return }
                do {
                    let info = try result.get()
                    self.encoding = info["encoding"] as? String ?? self.encoding
                    self.logicalSize = info["size"] as? [Int] ?? self.logicalSize
                    self.scaledCpuCanvas = info["scaledCpuCanvas"] as? Bool == true
                    self.ready = true
                    self.display(info)
                    self.syncActivity()
                    link.add(to: .main, forMode: .common)
                } catch { self.fail(error) }
            }
        }
    }
    func resize(_ size: CGSize, _ scale: CGFloat) {
        viewSize = size
        self.scale = scale
        let width = UInt32(size.width * scale)
        let height = UInt32(size.height * scale)
        guard ready else { return }
        let state = windowState?() ?? (windowed: false, resizable: false)
        var rect = [Int32](repeating: 0, count: 4)
        invoke { cmvs_apple_resize($0, width, height, state.windowed, state.resizable, &rect) }
        let frame = CGRect(x: CGFloat(rect[0]) / scale, y: CGFloat(rect[1]) / scale,
                           width: CGFloat(rect[2]) / scale, height: CGFloat(rect[3]) / scale)
        viewportFrame = frame
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        cpu.contentsScale = scale
        cpu.frame = scaledCpuCanvas
            ? CGRect(x: 0, y: 0, width: CGFloat(width) / scale, height: CGFloat(height) / scale)
            : frame
        CATransaction.commit()
        if canvasFrame != frame {
            Task { @MainActor [weak self] in
                guard let self, !self.closed, self.canvasFrame != self.viewportFrame else { return }
                self.canvasFrame = self.viewportFrame
            }
        }
    }
    func canvasRect(_ rect: [Int]) -> CGRect {
        // Read the @Observable trigger while using the current input viewport.
        _ = canvasFrame
        let canvas = viewportFrame
        guard rect.count == 4, logicalSize.count == 2, logicalSize[0] > 0, logicalSize[1] > 0 else { return .zero }
        let sx = canvas.width / CGFloat(logicalSize[0])
        let sy = canvas.height / CGFloat(logicalSize[1])
        return CGRect(x: canvas.minX + CGFloat(rect[0]) * sx, y: canvas.minY + CGFloat(rect[1]) * sy,
                      width: CGFloat(rect[2]) * sx, height: CGFloat(rect[3]) * sy)
    }

    func scene(_ value: Bool) {
        active = value
        syncActivity()
    }
    func focus(_ value: Bool) {
        focused = value
        syncActivity()
    }
    func measurePerformance(_ enabled: Bool) {
        measuringPerformance = enabled
        sampleStart = nil
        framesPerSecond = nil
    }
    private func samplePerformance(at time: CFTimeInterval) {
        guard measuringPerformance else { return }
        if let start = sampleStart {
            let elapsed = time - start.time
            guard elapsed >= 1 else { return }
            framesPerSecond = Double(frames - start.frames) / elapsed
        }
        sampleStart = (time, frames)
    }
    private var paused: Bool { !active || !focused || interrupted || dialog != nil || error != nil }
    var canFastForward: Bool { ready && !closed && !paused }

    func startDebugRecording() {
        guard ready, !closed, error == nil, debugRecording == nil else { return }
        #if os(iOS)
            let platform = "iOS"
            let gestures: Bool? = game.options.gestures
        #else
            let platform = "macOS"
            let gestures: Bool? = nil
        #endif
        debugReport = nil
        debugRecording = DebugRecording(environment: .init(
            platform: platform, metal: game.options.metal,
            nativeAudio: game.options.native_audio, multiTouchGestures: gestures))
    }

    func stopDebugRecording() {
        guard let debugRecording else { return }
        debugReport = debugRecording.finish()
        self.debugRecording = nil
    }

    private func syncActivity() {
        let paused = self.paused
        previous = nil
        sampleStart = nil
        framesPerSecond = nil
        if paused {
            lockedFastForward.removeAll()
            keyboardFastForward.removeAll()
            clearKeys?()
        }
        clock?.isPaused = paused
        if ready { invoke { cmvs_apple_pause($0, paused) } }
    }
    #if os(iOS)
        @objc private func audioInterrupted(_ note: Notification) {
            guard let kind = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt else { return }
            Task { @MainActor in
                guard !closed else { return }
                interruptionOngoing = kind == AVAudioSession.InterruptionType.began.rawValue
                interrupted = true
                syncActivity()
                let flags = note.userInfo?[AVAudioSessionInterruptionOptionKey] as? UInt ?? 0
                if !interruptionOngoing && !routePaused && AVAudioSession.InterruptionOptions(rawValue: flags).contains(.shouldResume) {
                    resumeAudio()
                } else if !interruptionOngoing { controls = true }
            }
        }
        @objc private func routeChanged(_ note: Notification) {
            guard let reason = note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt,
                reason == AVAudioSession.RouteChangeReason.oldDeviceUnavailable.rawValue
            else { return }
            Task { @MainActor in
                guard !closed else { return }
                routePaused = true
                interrupted = true
                controls = true
                syncActivity()
            }
        }
        func resumeAudio() {
            guard interrupted, !interruptionOngoing, !closed else { return }
            do {
                try AVAudioSession.sharedInstance().setActive(true)
                try engine.queue.sync {
                    if let api = engine.api { try api.check(cmvs_apple_restart_audio(api.handle)) }
                }
                routePaused = false
                interrupted = false
                audioError = nil
                syncActivity()
            } catch {
                controls = true
                audioError = error.localizedDescription
            }
        }
    #endif
    private func tick(_ delta: UInt32) throws -> (Int32, CGImage?, [[String: Any]]) {
        let recording = debugRecording
        let start = recording.map { _ in ProcessInfo.processInfo.systemUptime }
        var trace = CmvsAppleTickInput()
        var status: Int32 = -1
        defer {
            if let recording, let start {
                let elapsed = Int((ProcessInfo.processInfo.systemUptime - start) * 1_000_000)
                let input = trace.sampled ? recording.sample(
                    primaryHeld: trace.primary_held, touchPositionSampled: trace.touch_position_sampled) : nil
                recording.append(.tick(frame: frames + 1, deltaMs: delta, elapsedUs: elapsed,
                                       status: .init(rawValue: status) ?? .failed, input: input), at: start)
            }
        }
        return try engine.queue.sync {
            guard let api = engine.api else { throw PlayerError("Game closed.") }
            status = recording == nil
                ? cmvs_apple_tick(api.handle, delta) : cmvs_apple_tick_traced(api.handle, delta, &trace)
            if status < 0 {
                let fallback = PlayerError(engineMessage: api.text)
                if cmvs_apple_next_event(api.handle) == 1,
                   let stop = try? JSONDecoder().decode(StopDiagnostic.self, from: Data(api.text.utf8)) {
                    throw stop
                }
                throw fallback
            }
            var events = [[String: Any]]()
            while cmvs_apple_next_event(api.handle) == 1 { events.append(try api.object()) }
            let image = status == 1 && !game.options.metal ? try engine.image(presentation: scaledCpuCanvas) : nil
            return (status, image, events)
        }
    }
    @objc func step(_ link: CADisplayLink) {
        guard ready, !closed else { return }
        let start = previous ?? link.timestamp - 0.016
        previous = start
        let delta = UInt32(min(250, max(0, (link.timestamp - start) * 1000)))
        do {
            let result = try tick(delta)
            if result.0 == 2 {
                finish()
                return
            }
            if result.0 == 1 {
                previous = link.timestamp
                frames += 1
                if let image = result.1 { cpu.contents = image }
            }
            samplePerformance(at: link.timestamp)
            for e in result.2 {
                switch e["kind"] as? String {
                case "audioPaused":
                    interrupted = true
                    routePaused = true
                    controls = true
                    syncActivity()
                case "title": title = e["text"] as? String ?? title
                case "display": display(e)
                case "layout": resize(viewSize, scale)
                case "windowSize":
                    if let size = e["size"] as? [Int], size.count == 2 {
                        resizeWindow?(CGSize(width: size[0], height: size[1]))
                    }
                case "visibility": cursorVisible?(e["visible"] as? Bool ?? true)
                case "warp":
                    if let xy = e["point"] as? [Double], xy.count == 2 { warp?(CGPoint(x: xy[0], y: xy[1])) }
                case "cursor": cursor?(e["cursor"] as? [String: Any])
                case "font":
                    let name = e["face"] as? String
                    if let id = e["fontID"] as? String { dialogFont = fonts.first { $0.id == id } }
                    else if e["fontID"] != nil { dialogFont = nil }
                    else if e["default"] as? Bool == true { dialogFont = font }
                    else if let name { dialogFont = fonts.first { $0.name == name || $0.family == name } }
                    else { dialogFont = nil }
                    fallthrough
                case "message", "comment":
                    controls = false
                    draft = e["text"] as? String ?? ""
                    dialog = e
                    syncActivity()
                default: break
                }
            }
            if result.0 == 1 && !paused && !controls {
                accessibility.update(at: link.timestamp, engine: engine, cpuFrame: scaledCpuCanvas ? nil : result.1)
            }
            if smokeFrames > 0 && frames == smokeFrames {
                if let path = ProcessInfo.processInfo.environment["CMVS_SMOKE_OUTPUT"] {
                    try engine.queue.sync { try engine.screenshot(to: URL(fileURLWithPath: path)) }
                }
                NSLog("CMVS_SMOKE_OK frames=%d", frames)
                finish()
            }
        } catch { fail(error) }
    }
    private func display(_ info: [String: Any]) {
        let filter: CALayerContentsFilter = info["linear"] as? Bool == true ? .linear : .nearest
        cpu.magnificationFilter = filter
        cpu.minificationFilter = filter
        resize(viewSize, scale)
        if let value = info["fullscreen"] as? Bool { fullscreen?(value) }
    }
    func invoke(_ f: (OpaquePointer) -> Int32) {
        guard ready, !closed else { return }
        do { try engine.queue.sync { if let api = engine.api { try api.check(f(api.handle)) } } } catch { fail(error) }
    }
    #if os(iOS)
        func touch(_ id: UInt64, phase: DebugRecording.Phase, x: CGFloat, y: CGFloat) {
            guard ready, !closed else { return }
            debugRecording?.dispatch(id, phase: phase)
            invoke { cmvs_apple_touch($0, id, phase.nativeCode, x, y) }
        }
    #endif
    func key(_ code: UInt32, _ down: Bool) {
        if let modifier = FastForwardKey(rawValue: code) {
            guard canFastForward else { return }
            if down { keyboardFastForward.insert(modifier) } else { keyboardFastForward.remove(modifier) }
            if lockedFastForward.contains(modifier) { return }
        }
        invoke { cmvs_apple_key($0, code, down) }
    }
    func pulse(_ code: UInt32) {
        key(code, true)
        key(code, false)
    }
    func setFastForward(_ modifier: FastForwardKey, locked: Bool) {
        guard canFastForward else { return }
        if locked { lockedFastForward.insert(modifier) } else { lockedFastForward.remove(modifier) }
        if !keyboardFastForward.contains(modifier) {
            invoke { cmvs_apple_key($0, modifier.rawValue, locked) }
        }
    }
    func capture() {
        guard canCapture else { return }
        do {
            let dir = Library.userFiles.appending(path: "Screenshots")
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let stamp = Date().ISO8601Format(.iso8601(timeZone: .gmt, includingFractionalSeconds: true))
                .replacingOccurrences(of: ":", with: "-")
            let name = "\(game.filename)-\(stamp).png"
            try engine.queue.sync { try engine.screenshot(to: dir.appending(path: name)) }
            screenshotName = name
        } catch { captureError = error.localizedDescription }
    }
    struct CommentStatus {
        let characters: Int
        let limit: Int
        let bytes: Int?
        let byteLimit: Int
        let problem: String?
    }
    var commentStatus: CommentStatus {
        let limit = dialog?["limit"] as? Int ?? 0
        let byteLimit = dialog?["bytes"] as? Int ?? 0
        let characters = draft.unicodeScalars.count
        let control = draft.unicodeScalars.contains { $0.properties.generalCategory == .control }
        let bytes: Int? = control ? nil : engine.queue.sync {
            guard let api = engine.api else { return nil }
            let size = cmvs_apple_comment_bytes(api.handle, draft)
            return size < 0 ? nil : Int(size)
        }
        let problem: String?
        if control { problem = String(localized: "Remove control characters.") }
        else if characters > limit { problem = String(localized: "The comment has too many characters.") }
        else if let bytes {
            problem = bytes > byteLimit ? String(localized: "The encoded comment exceeds the byte limit.") : nil
        } else { problem = String(localized: "Some characters cannot be encoded in \(encoding).") }
        return CommentStatus(characters: characters, limit: limit, bytes: bytes, byteLimit: byteLimit, problem: problem)
    }

    func fastForwardBinding(_ key: FastForwardKey) -> Binding<Bool> {
        Binding(get: { self.lockedFastForward.contains(key) }, set: { self.setFastForward(key, locked: $0) })
    }

    func answer(_ accepted: Bool, face: FontFace? = nil) {
        guard let e = dialog else { return }
        switch e["kind"] as? String {
        case "comment":
            guard !accepted || commentStatus.problem == nil else { return }
            let id = (e["id"] as? NSNumber)?.uint32Value ?? 0
            invoke { accepted ? cmvs_apple_comment($0, id, draft) : cmvs_apple_comment($0, id, nil) }
        case "font":
            invoke { handle in
                if accepted, let face { return cmvs_apple_font(handle, face.id) }
                return cmvs_apple_font(handle, nil)
            }
        default: invoke { cmvs_apple_message($0, accepted) }
        }
        dialog = nil
        syncActivity()
    }
    func fail(_ cause: Error) {
        debugRecording?.append(.playbackFailed)
        stopDebugRecording()
        if smokeFrames > 0 { NSLog("CMVS_SMOKE playback failed: %@", cause.localizedDescription) }
        clock?.isPaused = true
        if ready { engine.queue.sync { if let api = engine.api { _ = cmvs_apple_pause(api.handle, true) } } }
        error = cause.localizedDescription
    }
    func close() {
        guard !closed else { return }
        closed = true
        debugRecording = nil
        debugReport = nil
        accessibility.enable(false)
        clock?.invalidate()
        clock = nil
        NotificationCenter.default.removeObserver(self)
        defer { quit = nil }
        if !ready {
            let engine = engine
            let report = closeError
            let didClose = didClose
            engine.queue.async {
                do { try engine.close(save: false) }
                catch { DispatchQueue.main.async { report?(error.localizedDescription) } }
                DispatchQueue.main.async { didClose?() }
            }
        } else {
            do {
                try engine.queue.sync {
                    if error == nil && frames > 0 {
                        try? engine.screenshot(to: game.thumbnail)
                    }
                    try engine.close(save: error == nil)
                }
            } catch { closeError?(error.localizedDescription) }
            didClose?()
        }
        ready = false
        #if os(iOS)
            if game.options.native_audio {
                try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
            }
        #endif
    }
    func finish() {
        let exit = quit
        close()
        exit?()
    }
}
