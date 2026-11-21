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
    private var selectedFaces = [String: FontFace]()
    private(set) var dialogFont: FontFace?
    let fonts: [FontFace]
    let engine = Engine()
    let accessibility = GameAccessibility()
    let cpu = CALayer()
    private var clock: CADisplayLink?
    private var previous: CFTimeInterval?
    private var frames = 0
    @ObservationIgnored private var measuringPerformance = false
    @ObservationIgnored private var sampleStart: (time: CFTimeInterval, frames: Int)?
    private(set) var framesPerSecond: Double?
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
    var error: String?
    var dialog: [String: Any]?
    var draft = ""
    var controls = false
    private(set) var lockedFastForward = Set<FastForwardKey>()
    @ObservationIgnored private var keyboardFastForward = Set<FastForwardKey>()
    var fullscreen: ((Bool) -> Void)?
    var cursorVisible: ((Bool) -> Void)?
    var cursor: (([String: Any]?) -> Void)?
    var closeError: ((String) -> Void)?
    var quit: (() -> Void)?
    var encoding: String
    private var mode = 1
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
        engine.queue.async {
            let result = Result { try engine.open(game, font: font, layer: layer, width: width, height: height) }
            DispatchQueue.main.async {
                guard !self.closed else { return }
                do {
                    let info = try result.get()
                    self.encoding = info["encoding"] as? String ?? self.encoding
                    self.logicalSize = info["size"] as? [Int] ?? self.logicalSize
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
        if ready { invoke { cmvs_apple_resize($0, width, height) } }
        let w = CGFloat(logicalSize[0])
        let h = CGFloat(logicalSize[1])
        var factor = min(size.width / w, size.height / h)
        if mode == 2 { factor = max(size.width / w, size.height / h) }
        if mode == 0 { factor = min(1 / scale, factor) }
        let target = mode == 3 ? size : CGSize(width: w * factor, height: h * factor)
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        cpu.frame = CGRect(
            x: (size.width - target.width) / 2, y: (size.height - target.height) / 2, width: target.width,
            height: target.height)
        CATransaction.commit()
    }
    func canvasRect(_ rect: [Int]) -> CGRect {
        let canvas = cpu.frame
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
    @objc func step(_ link: CADisplayLink) {
        guard ready, !closed else { return }
        let start = previous ?? link.timestamp - 0.016
        previous = start
        let delta = UInt32(min(250, max(0, (link.timestamp - start) * 1000)))
        do {
            let result = try engine.queue.sync { () -> (Int32, CGImage?, [[String: Any]]) in
                guard let api = engine.api else { throw PlayerError("Game closed.") }
                let result = cmvs_apple_tick(api.handle, delta)
                if result < 0 {
                    let fallback = PlayerError(engineMessage: api.text)
                    if cmvs_apple_next_event(api.handle) == 1,
                       let stop = try? JSONDecoder().decode(StopDiagnostic.self, from: Data(api.text.utf8)) {
                        throw stop
                    }
                    throw fallback
                }
                var events = [[String: Any]]()
                while cmvs_apple_next_event(api.handle) == 1 { events.append(try api.object()) }
                let image = result == 1 && !game.options.metal ? try engine.image() : nil
                return (result, image, events)
            }
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
                case "visibility": cursorVisible?(e["visible"] as? Bool ?? true)
                case "warp":
                    if let xy = e["point"] as? [Double], xy.count == 2 { warp?(CGPoint(x: xy[0], y: xy[1])) }
                case "cursor": cursor?(e["cursor"] as? [String: Any])
                case "font":
                    let name = e["face"] as? String
                    if let name, let selected = selectedFaces[name] { dialogFont = selected }
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
                accessibility.update(at: link.timestamp, engine: engine, cpuFrame: result.1)
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
        mode = info["mode"] as? Int ?? 1
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
            var key: String?
            invoke { handle in
                if let face {
                    let code = cmvs_apple_font(handle, face.url.path, face.index)
                    if code == 0 { key = String(cString: cmvs_apple_text(handle)) }
                    return code
                }
                return cmvs_apple_font(handle, nil, 0)
            }
            if let key, let face { selectedFaces[key] = face }
        default: invoke { cmvs_apple_message($0, accepted) }
        }
        dialog = nil
        syncActivity()
    }
    func fail(_ cause: Error) {
        if smokeFrames > 0 { NSLog("CMVS_SMOKE playback failed: %@", cause.localizedDescription) }
        clock?.isPaused = true
        if ready { engine.queue.sync { if let api = engine.api { _ = cmvs_apple_pause(api.handle, true) } } }
        error = cause.localizedDescription
    }
    func close() {
        guard !closed else { return }
        closed = true
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
                do { try engine.close() }
                catch { DispatchQueue.main.async { report?(error.localizedDescription) } }
                DispatchQueue.main.async { didClose?() }
            }
        } else {
            do {
                try engine.queue.sync {
                    if frames > 0 {
                        try? engine.screenshot(to: game.thumbnail)
                    }
                    try engine.close()
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
