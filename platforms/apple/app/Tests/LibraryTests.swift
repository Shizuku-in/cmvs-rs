import CoreTransferable
import Foundation
import XCTest

@testable import CMVS

final class LibraryTests: XCTestCase {
    func testSaveArchiveSuppliesZIPFilenameToExporter() throws {
        guard #available(iOS 18.2, macOS 15.2, *) else { throw XCTSkip("Requires Transferable.suggestedFilename") }
        let info = GameInfo(profile: "", family: "CMVS", basis: "", encoding: "gbk", size: [], savedata: false)
        let game = Game(id: UUID(), name: "Example", info: info, options: Options())
        let archive = try SaveArchive(game: game)
        XCTAssertEqual(archive.suggestedFilename, "Example.zip")
    }

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

    func testMigratingTwoEditionsPreservesTheirSeparateProgress() throws {
        let fm = FileManager.default
        let root = fm.temporaryDirectory.appending(path: UUID().uuidString)
        defer { try? fm.removeItem(at: root) }
        let saves = root.appending(path: "Saves")
        let info = GameInfo(game: "example", profile: "", family: "CMVS", basis: "", encoding: "gbk", size: [], savedata: false)
        var catalog = Library.Catalog()
        func addEdition(_ name: String) throws -> String {
            let source = root.appending(path: name)
            try fm.createDirectory(at: source, withIntermediateDirectories: true)
            let bookmark = try source.bookmarkData(options: .minimalBookmark, includingResourceValuesForKeys: nil, relativeTo: nil)
            catalog.games.append(Game(id: UUID(), name: name, info: info, bookmark: bookmark, options: Options()))
            return Game.sourceKey(try XCTUnwrap(catalog.games.last).resolve())
        }
        let original = try addEdition("Original")
        let patched = try addEdition("Patched")
        func slot(_ folder: String) -> URL { saves.appending(path: folder + "/save001.dat") }
        for (folder, byte) in [(info.game!, UInt8(1)), (patched, UInt8(2))] {
            try fm.createDirectory(at: saves.appending(path: folder), withIntermediateDirectories: true)
            try Data([byte]).write(to: slot(folder))
        }
        XCTAssertEqual(Game.sourceKey(URL(fileURLWithPath: "/game")), "287421120eed3df4")
        XCTAssertTrue(try Library.migrateSaveFolders(&catalog, saves: saves))
        XCTAssertEqual(catalog.games.map(\.saveFolderName), ["source-" + original, "source-" + patched])
        for (game, byte) in zip(catalog.games, [UInt8(1), 2]) {
            XCTAssertEqual(try Data(contentsOf: slot(game.saveFolderName)), Data([byte]))
        }
        try Data([3]).write(to: slot("source-" + patched))
        XCTAssertFalse(try Library.migrateSaveFolders(&catalog, saves: saves))
        XCTAssertEqual(try Data(contentsOf: slot("source-" + patched)), Data([3]))
        XCTAssertEqual(try Data(contentsOf: slot(patched)), Data([2]))
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

    func testSystemFontsCanBeSelectedWithoutImporting() throws {
        let fm = FileManager.default
        let directory = fm.temporaryDirectory.appending(path: UUID().uuidString)
        defer { try? fm.removeItem(at: directory) }
        let library = Library(directory: directory)
        library.systemFonts = Library.scanFonts()
        let first = try XCTUnwrap(library.systemFonts.first)
        XCTAssertEqual(library.defaultFont, first)
        XCTAssertNotNil(first.preview(size: 18))
        XCTAssertEqual(Set(library.systemFonts.map(\.id)).count, library.systemFonts.count)

        let selected = try XCTUnwrap(library.systemFonts.last)
        library.data.defaults.font = selected.id
        library.persist()
        library.reload()
        let info = GameInfo(profile: "", family: "CMVS", basis: "", encoding: "gbk", size: [], savedata: false)
        let game = Game(id: UUID(), name: "Example", info: info, options: Options())
        XCTAssertEqual(try library.font(for: game), selected)
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

    func testDependencyCatalogParsesPackagesAndExpandsSharedLicenses() {
        let raw = """
            Dependency licenses for linked Apple target crates.

            === PACKAGES ===

            == ogg 0.8.0 — BSD-3-Clause ==

            Copyright (c) 2016-2017 est31

            == ogg 0.9.2 — BSD-3-Clause ==

            Copyright (c) 2016-2017 est31

            == zip 7.2.0 — MIT ==
            Shared: MIT

            Copyright (c) 2014 Mathijs van de Nes

            === SHARED LICENSES ===

            == [MIT] ==

            Permission is hereby granted, free of charge.
            """
        let catalog = DependencyCatalog.parse(raw)
        XCTAssertEqual(catalog.packages.count, 3)
        XCTAssertEqual(catalog.packages[0].id, "ogg@0.8.0")
        XCTAssertEqual(catalog.packages[1].id, "ogg@0.9.2")
        XCTAssertEqual(catalog.packages[2].shared, ["MIT"])
        XCTAssertEqual(catalog.shared, [.init(name: "MIT", text: "Permission is hereby granted, free of charge.")])

        let detail = catalog.detailText(for: catalog.packages[2])
        XCTAssertTrue(detail.contains("zip 7.2.0 — MIT"))
        XCTAssertTrue(detail.contains("Copyright (c) 2014 Mathijs van de Nes"))
        XCTAssertTrue(detail.contains("--- MIT ---"))
        XCTAssertTrue(detail.contains("Permission is hereby granted, free of charge."))
    }
}
