import Foundation
import XCTest

@testable import Cmvs

final class LibraryTests: XCTestCase {
    func testLegacySavesImportedOnlyOnce() throws {
        let fm = FileManager.default
        let directory = fm.temporaryDirectory.appending(path: UUID().uuidString)
        defer { try? fm.removeItem(at: directory) }
        // FNV-1a of "/game" in the UIKit prototype.
        let source = URL(fileURLWithPath: "/game")
        let old = directory.appending(path: "287421120eed3df4")
        let current = directory.appending(path: "example")
        try fm.createDirectory(at: old, withIntermediateDirectories: true)
        try Data([1]).write(to: old.appending(path: "save001.dat"))
        try Game.migrateSaves(source: source, destination: current)
        let slot = current.appending(path: "save001.dat")
        XCTAssertEqual(try Data(contentsOf: slot), Data([1]))
        try Data([2]).write(to: slot)
        try Game.migrateSaves(source: source, destination: current)
        XCTAssertEqual(try Data(contentsOf: slot), Data([2]))
        XCTAssertEqual(try Data(contentsOf: old.appending(path: "save001.dat")), Data([1]))
    }

    func testImportRejectsSymlinkBeforeCopying() throws {
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appending(path: UUID().uuidString)
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: dir) }
        try fm.createSymbolicLink(at: dir.appending(path: "cycle"), withDestinationURL: dir)
        XCTAssertThrowsError(
            try Library.copyGame(
                dir, dir.deletingLastPathComponent().appending(path: UUID().uuidString), Progress(totalUnitCount: -1))
        ) {
            XCTAssertTrue($0 is PlayerError)
        }
    }

    func testBookmarkedFolderSurvivesCatalogRoundTrip() throws {
        let dir = FileManager.default.temporaryDirectory
        let bookmark = try dir.bookmarkData(
            options: .minimalBookmark, includingResourceValuesForKeys: nil, relativeTo: nil)
        let info = GameInfo(
            profile: "example", family: "CMVS", basis: "Explicit", encoding: "gbk", size: [640, 480], savedata: false)
        var options = Options()
        options.encoding = "gbk"
        let game = Game(id: UUID(), name: "Example", info: info, bookmark: bookmark, options: options)
        let restored = try JSONDecoder().decode(Game.self, from: JSONEncoder().encode(game))
        XCTAssertEqual(try restored.resolve().resolvingSymlinksInPath().path, dir.resolvingSymlinksInPath().path)
        XCTAssertEqual(restored.saves, game.saves)
        XCTAssertEqual(restored.options.encoding, "gbk")
    }
}
