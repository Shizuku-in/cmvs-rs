import Foundation

final class DebugReport: Encodable, Sendable {
    struct Environment: Encodable, Sendable {
        let platform: String
        let osVersion: String
        let appVersion: String?
        let buildNumber: String?
        let renderer: String
        let nativeAudio: Bool
        let multiTouchGestures: Bool?

        init(platform: String, metal: Bool, nativeAudio: Bool, multiTouchGestures: Bool?) {
            self.platform = platform
            let os = ProcessInfo.processInfo.operatingSystemVersion
            osVersion = "\(os.majorVersion).\(os.minorVersion).\(os.patchVersion)"
            appVersion = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
            buildNumber = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String
            renderer = metal ? "Metal" : "CPU"
            self.nativeAudio = nativeAudio
            self.multiTouchGestures = multiTouchGestures
        }
    }

    struct Summary: Encodable, Sendable {
        let durationMs: Int
        let eventCount: Int
        let completedFrames: Int
    }

    let schemaVersion = 1
    let environment: Environment
    let summary: Summary
    let events: [DebugRecording.Event]
    private let directory = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)

    private enum CodingKeys: String, CodingKey { case schemaVersion, environment, summary, events }

    init(environment: Environment, events: [DebugRecording.Event], endMs: Int) {
        self.environment = environment
        self.events = events
        let completed = events.reduce(into: 0) { count, event in
            if case .tick(_, _, _, .completed, _) = event.detail { count += 1 }
        }
        summary = Summary(
            durationMs: endMs - (events.first?.timeMs ?? endMs), eventCount: events.count, completedFrames: completed)
    }

    deinit { try? FileManager.default.removeItem(at: directory) }

    func exportFile() throws -> URL {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        let data = try encoder.encode(self)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appending(path: "cmvs-debug.json")
        try data.write(to: url, options: .atomic)
        return url
    }
}

final class DebugRecording {
    enum Phase: String, Encodable, Sendable {
        case began, moved, ended, cancelled

        var nativeCode: UInt32 {
            switch self {
            case .began: 0
            case .moved: 1
            case .ended: 2
            case .cancelled: 3
            }
        }
    }

    enum Gesture: String, Encodable, Sendable {
        case deferred, startedOnMovement, startedOnTimer, startedOnRelease
        case cancelledBySecondContact, cancelledBySystem, cancelledOnPause
    }

    enum TickStatus: Int32, Encodable, Sendable {
        case failed = -1, budgetSlice = 0, completed = 1, quit = 2, inactive = 3
    }

    struct InputSample: Encodable, Sendable {
        let primaryHeld: Bool
        let touchPositionSampled: Bool
        let lastDispatch: Int?
    }

    enum Detail: Encodable, Sendable {
        case touch(phase: Phase, contacts: Int, dxPoints: Int?, dyPoints: Int?)
        case gesture(decision: Gesture)
        case dispatch(phase: Phase, sourceEvent: Int?)
        case tick(frame: Int, deltaMs: UInt32, elapsedUs: Int, status: TickStatus, input: InputSample?)
        case playbackFailed
    }

    struct Event: Encodable, Sendable {
        let sequence: Int
        let timeMs: Int
        let detail: Detail
    }

    private let environment: DebugReport.Environment
    private let start: TimeInterval
    private var buffer = [Event?](repeating: nil, count: 8192)
    private var first = 0
    private var next = 0
    // UITouch identities stay in memory only. The report links events by sequence number.
    private var sourceEvents: [UInt64: Int] = [:]
    private var lastDispatch: Int?

    init(environment: DebugReport.Environment, at time: TimeInterval = ProcessInfo.processInfo.systemUptime) {
        self.environment = environment
        start = time
    }

    func receive(_ id: UInt64, phase: Phase, contacts: Int, dx: Int? = nil, dy: Int? = nil) {
        sourceEvents[id] = append(.touch(phase: phase, contacts: contacts, dxPoints: dx, dyPoints: dy))
    }

    func forget(_ id: UInt64) {
        sourceEvents.removeValue(forKey: id)
    }

    func dispatch(_ id: UInt64, phase: Phase) {
        let source = sourceEvents[id].flatMap { $0 >= first ? $0 : nil }
        lastDispatch = append(.dispatch(phase: phase, sourceEvent: source))
    }

    func sample(primaryHeld: Bool, touchPositionSampled: Bool) -> InputSample {
        InputSample(
            primaryHeld: primaryHeld, touchPositionSampled: touchPositionSampled,
            lastDispatch: lastDispatch.flatMap { $0 >= first ? $0 : nil })
    }

    @discardableResult func append(_ detail: Detail, at time: TimeInterval = ProcessInfo.processInfo.systemUptime) -> Int {
        let ms = milliseconds(sinceStartAt: time)
        trim(at: ms)
        if next - first == buffer.count { first += 1 }
        let sequence = next
        buffer[sequence % buffer.count] = Event(sequence: sequence, timeMs: ms, detail: detail)
        next += 1
        return sequence
    }

    func finish(at time: TimeInterval = ProcessInfo.processInfo.systemUptime) -> DebugReport {
        let ms = milliseconds(sinceStartAt: time)
        trim(at: ms)
        let events = (first..<next).compactMap { buffer[$0 % buffer.count] }
        return DebugReport(environment: environment, events: events, endMs: ms)
    }

    private func milliseconds(sinceStartAt time: TimeInterval) -> Int {
        Int(max(0, (time - start) * 1000))
    }

    private func trim(at ms: Int) {
        while first < next, let event = buffer[first % buffer.count], event.timeMs < ms - 30_000 {
            buffer[first % buffer.count] = nil
            first += 1
        }
    }
}
