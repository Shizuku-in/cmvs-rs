import Foundation
import XCTest

@testable import CMVS

final class LibraryTests: XCTestCase {
    func testUnreadableCatalogIsPreservedUntilRetrySucceeds() throws {
        let fm = FileManager.default
        let directory = fm.temporaryDirectory.appending(path: UUID().uuidString)
        defer { try? fm.removeItem(at: directory) }
        try fm.createDirectory(at: directory, withIntermediateDirectories: true)
        let index = directory.appending(path: "library.json")
        let original = Data("damaged index".utf8)
        try original.write(to: index)
        let library = Library(directory: directory)
        XCTAssertNotNil(library.loadError)
        library.persist()
        XCTAssertEqual(try Data(contentsOf: index), original)
        var catalog = Library.Catalog()
        catalog.defaults.metal = false
        try JSONEncoder().encode(catalog).write(to: index)
        library.reload()
        XCTAssertNil(library.loadError)
        XCTAssertEqual(library.data, catalog)
    }

    func testCatalogMigrationResumesAndLeavesUserFilesInDocuments() throws {
        let fm = FileManager.default
        let root = fm.temporaryDirectory.appending(path: UUID().uuidString)
        defer { try? fm.removeItem(at: root) }
        let documents = root.appending(path: "Documents")
        let support = root.appending(path: "Support")
        let info = GameInfo(profile: "", family: "CMVS", basis: "", encoding: "gbk", size: [], savedata: false)
        let game = Game(id: UUID(), name: "Game", info: info, options: Options())
        let font = FontFace(file: "Fonts/imported.ttc", index: 0, name: "Font", bytes: 1)
        let catalog = Library.Catalog(games: [game], fonts: [font])
        let copy = "Games/\(game.id)/data/pack"
        let cover = "\(game.id).png"
        let publicFiles = ["Saves/Game/save001.dat", "Screenshots/game.png", "Games/my-game", "cover.png"]
        func write(_ path: String, under base: URL, _ byte: UInt8) throws {
            let file = base.appending(path: path)
            try fm.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
            try Data([byte]).write(to: file)
        }

        // The cover moved before the previous launch was interrupted.
        try write(cover, under: support, 2)
        try write(copy, under: documents, 1)
        for path in publicFiles + [font.file] { try write(path, under: documents, 2) }
        let index = try JSONEncoder().encode(catalog)
        try index.write(to: documents.appending(path: "library.json"))
        try Library.migrateCatalog(from: documents, to: support)
        try Library.migrateCatalog(from: documents, to: support)

        XCTAssertEqual(try Data(contentsOf: support.appending(path: "library.json")), index)
        XCTAssertEqual(try Data(contentsOf: support.appending(path: copy)), Data([1]))
        for path in [cover, font.file] {
            XCTAssertEqual(try Data(contentsOf: support.appending(path: path)), Data([2]))
        }
        for path in ["library.json", cover, "Fonts", copy] {
            XCTAssertFalse(fm.fileExists(atPath: documents.appending(path: path).path))
        }
        for path in publicFiles {
            XCTAssertEqual(try Data(contentsOf: documents.appending(path: path)), Data([2]))
        }
    }

    func testCatalogMigrationHandlesExistingDestinationItems() throws {
        let fm = FileManager.default
        let root = fm.temporaryDirectory.appending(path: UUID().uuidString)
        defer { try? fm.removeItem(at: root) }
        let documents = root.appending(path: "Documents")
        let support = root.appending(path: "Support")
        let info = GameInfo(profile: "", family: "CMVS", basis: "", encoding: "gbk", size: [], savedata: false)
        let game = Game(id: UUID(), name: "Game", info: info, options: Options())
        let font = FontFace(file: "Fonts/imported.ttc", index: 0, name: "Font", bytes: 1)
        let catalog = Library.Catalog(games: [game], fonts: [font])
        let pack = "Games/\(game.id)/data/pack"
        let extra = "Games/\(game.id)/data/extra"
        let cover = "\(game.id).png"
        func write(_ path: String, under base: URL, _ byte: UInt8) throws {
            let file = base.appending(path: path)
            try fm.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
            try Data([byte]).write(to: file)
        }

        // Simulate conflicting cover, font, and partial directory already in Support.
        try write(cover, under: support, 9)
        try write(cover, under: documents, 2)
        try write(font.file, under: support, 9)
        try write(font.file, under: documents, 3)
        try write(extra, under: support, 4)
        try write(pack, under: documents, 1)
        let index = try JSONEncoder().encode(catalog)
        try index.write(to: documents.appending(path: "library.json"))

        try Library.migrateCatalog(from: documents, to: support)

        XCTAssertEqual(try Data(contentsOf: support.appending(path: "library.json")), index)
        XCTAssertEqual(try Data(contentsOf: support.appending(path: cover)), Data([2]))
        XCTAssertEqual(try Data(contentsOf: support.appending(path: font.file)), Data([3]))
        XCTAssertEqual(try Data(contentsOf: support.appending(path: pack)), Data([1]))
        XCTAssertEqual(try Data(contentsOf: support.appending(path: extra)), Data([4]))
        XCTAssertFalse(fm.fileExists(atPath: documents.appending(path: "library.json").path))
        XCTAssertFalse(fm.fileExists(atPath: documents.appending(path: "Games").path))
        XCTAssertFalse(fm.fileExists(atPath: documents.appending(path: "Fonts").path))
    }

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

    func testRemovingGameDeletesOnlyOwnedFiles() throws {
        let fm = FileManager.default
        let directory = fm.temporaryDirectory.appending(path: UUID().uuidString)
        defer { try? fm.removeItem(at: directory) }
        let library = Library(directory: directory)
        let info = GameInfo(profile: "", family: "CMVS", basis: "", encoding: "gbk", size: [800, 600], savedata: false)
        var game = Game(id: UUID(), name: "Example", info: info, options: Options())
        let copy = try game.resolve(in: directory)
        try fm.createDirectory(at: copy, withIntermediateDirectories: true)
        let cover = game.thumbnail(in: directory)
        try Data([1]).write(to: cover)
        let source = fm.temporaryDirectory.appending(path: UUID().uuidString)
        try fm.createDirectory(at: source, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: source) }

        game.bookmark = try source.bookmarkData(options: .minimalBookmark, includingResourceValuesForKeys: nil, relativeTo: nil)
        library.data.games = [game]
        try library.remove(game)
        XCTAssertTrue(library.data.games.isEmpty)
        XCTAssertFalse(fm.fileExists(atPath: cover.path))
        XCTAssertTrue(fm.fileExists(atPath: source.path))
        XCTAssertTrue(fm.fileExists(atPath: copy.path))

        game.bookmark = nil
        library.data.games = [game]
        try library.remove(game)
        XCTAssertFalse(fm.fileExists(atPath: copy.path))
    }

    func testRemovingFontCollectionResetsFaceReferences() throws {
        let fm = FileManager.default
        let directory = fm.temporaryDirectory.appending(path: UUID().uuidString)
        defer { try? fm.removeItem(at: directory) }
        let library = Library(directory: directory)
        let face = FontFace(file: "Fonts/\(UUID()).ttc", index: 0, name: "Regular", bytes: 1)
        var bold = face
        bold.index = 1
        let fontURL = face.url(in: directory)
        try fm.createDirectory(at: fontURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data([1]).write(to: fontURL)
        let fallback = FontFace(file: "/system.ttf", index: 0, name: "Fallback", bytes: 1)
        library.systemFonts = [fallback]
        library.data.fonts = [face, bold]
        library.data.defaults.font = face.id
        var options = Options()
        options.font = bold.id
        let info = GameInfo(profile: "", family: "CMVS", basis: "", encoding: "gbk", size: [800, 600], savedata: false)
        library.data.games = [Game(id: UUID(), name: "Example", info: info, options: options)]

        try library.removeFont(bold)
        XCTAssertFalse(fm.fileExists(atPath: fontURL.path))
        XCTAssertTrue(library.data.fonts.isEmpty)
        XCTAssertEqual(library.data.defaults.font, "")
        XCTAssertEqual(library.data.games[0].options.font, "")
        XCTAssertEqual(try library.font(for: library.data.games[0]), fallback)
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
