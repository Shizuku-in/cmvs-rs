import Foundation
import XCTest

@testable import CMVS

final class DebugRecordingTests: XCTestCase {
    private let environment = DebugReport.Environment(
        platform: "iOS", metal: false, nativeAudio: true, multiTouchGestures: true)

    func testReportLinksTouchDispatchAndInputSample() throws {
        let recording = DebugRecording(environment: environment)
        let contact: UInt64 = 9_876_543_210
        recording.receive(contact, phase: .moved, contacts: 1, dx: 3, dy: -12)
        recording.dispatch(contact, phase: .moved)
        recording.append(.tick(
            frame: 7, deltaMs: 16, elapsedUs: 450, status: .completed,
            input: recording.sample(primaryHeld: true, touchPositionSampled: true)))
        let report = recording.finish()
        let data = try Data(contentsOf: report.exportFile())
        let json = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(Set(json.keys), ["schemaVersion", "environment", "summary", "events"])
        let events = try XCTUnwrap(json["events"] as? [[String: Any]])
        XCTAssertEqual(events.count, 3)
        let touch = try detail("touch", in: events[0])
        XCTAssertEqual(touch["dyPoints"] as? Int, -12)
        let dispatch = try detail("dispatch", in: events[1])
        XCTAssertEqual(dispatch["sourceEvent"] as? Int, events[0]["sequence"] as? Int)
        let tick = try detail("tick", in: events[2])
        let input = try XCTUnwrap(tick["input"] as? [String: Any])
        XCTAssertEqual(input["lastDispatch"] as? Int, events[1]["sequence"] as? Int)
        XCTAssertEqual(input["primaryHeld"] as? Bool, true)
        XCTAssertEqual(report.summary.completedFrames, 1)
        XCTAssertFalse(String(decoding: data, as: UTF8.self).contains(String(contact)))
    }

    func testRecentWindowAndCapacityKeepEventsInOrder() {
        let recording = DebugRecording(environment: environment, at: 0)
        recording.append(.gesture(decision: .deferred), at: 1)
        for _ in 0..<8200 { recording.append(.gesture(decision: .startedOnMovement), at: 32) }
        let full = recording.finish(at: 32)
        XCTAssertEqual(full.events.map(\.sequence), Array(9...8200))

        recording.append(.playbackFailed, at: 63)
        let recent = recording.finish(at: 64)
        XCTAssertEqual(recent.events.map(\.sequence), [8201])
        XCTAssertEqual(recent.summary.durationMs, 1000)
    }

    private func detail(_ name: String, in event: [String: Any]) throws -> [String: Any] {
        let detail = try XCTUnwrap(event["detail"] as? [String: Any])
        return try XCTUnwrap(detail[name] as? [String: Any])
    }
}
