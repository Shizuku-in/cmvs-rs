import CmvsAppleC
import CoreText
import Foundation
import Observation

struct PlayerError: LocalizedError {
    let errorDescription: String?
    init(_ message: String) { errorDescription = NSLocalizedString(message, comment: "") }

    init(engineMessage: String) {
        let missing = "\0"
        let message = Bundle.main.localizedString(forKey: engineMessage, value: missing, table: nil)
        errorDescription = message == missing ? String(localized: "Engine error: \(engineMessage)") : message
    }
}

struct GameInfo: Codable, Equatable {
    var game: String?
    var profile, family, basis, encoding: String
    var size: [Int]
    var savedata: Bool

    static let encodingLabels = ["Shift-JIS", "GBK", "UTF-8"]
    var encodingLabel: String { Self.encodingLabels.first { $0.lowercased() == encoding } ?? encoding }
}

struct Options: Codable, Equatable {
    var profile = ""
    var encoding = "auto"
    var font = ""
    var aspect: Int?
    var metal = true
    var native_audio = true
    var game_savedata = false
    var skip_auto_resume = false
    var block_presumed = false
    var block_unreviewed = false
    var continue_without_snapshot = false
    var gestures = true
}

struct Game: Identifiable, Codable, Equatable {
    var id: UUID
    var name: String
    var info: GameInfo
    var bookmark: Data?
    var options: Options
    var lastPlayed: Date?
    var saveFolder: String?
    var thumbnail: URL { Library.base.appending(path: "\(id).png") }
    var filename: String {
        let cleaned = name.components(separatedBy: CharacterSet(charactersIn: "/:\\").union(.controlCharacters))
            .joined(separator: "-").trimmingCharacters(in: CharacterSet(charactersIn: ". "))
        var stem = ""
        for ch in cleaned {
            guard stem.utf8.count + ch.utf8.count <= 180 else { break }
            stem.append(ch)
        }
        return stem.isEmpty ? "CMVS" : stem
    }

    func resolve() throws -> URL {
        var stale = false
        return try resolve(stale: &stale)
    }

    func resolve(stale: inout Bool) throws -> URL {
        guard let bookmark else { return Library.base.appending(path: "Games/\(id)") }
        #if os(macOS)
            let options: URL.BookmarkResolutionOptions = [.withoutUI]
        #else
            let options: URL.BookmarkResolutionOptions = []
        #endif
        return try URL(resolvingBookmarkData: bookmark, options: options, relativeTo: nil, bookmarkDataIsStale: &stale)
    }

    var saves: URL {
        let name = saveFolder ?? info.game ?? id.uuidString
        let safe =
            !name.isEmpty
            && name.utf8.allSatisfy {
                (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || $0 == 45 || $0 == 95
            }
        return Library.userFiles.appending(path: "Saves").appending(path: safe ? name : id.uuidString)
    }

    // The UIKit player keyed saves by the FNV-1a hash of the selected folder path.
    static func migrateSaves(source: URL, destination: URL) throws {
        let fm = FileManager.default
        let hash = source.path.utf8.reduce(UInt64(14_695_981_039_346_656_037)) {
            ($0 ^ UInt64($1)) &* 1_099_511_628_211
        }
        let parent = destination.deletingLastPathComponent()
        let old = parent.appending(path: String(format: "%016llx", hash))
        guard fm.fileExists(atPath: old.path), !fm.fileExists(atPath: destination.path) else { return }
        let staged = parent.appending(path: ".import-\(UUID())")
        defer { try? fm.removeItem(at: staged) }
        try fm.copyItem(at: old, to: staged)
        try fm.moveItem(at: staged, to: destination)
    }
}

struct FontFace: Identifiable, Codable, Hashable {
    var file: String
    var index: UInt32
    var name: String
    var bytes: Int
    var family: String?
    var id: String { "\(file)#\(index)" }
    var url: URL { file.hasPrefix("/") ? URL(fileURLWithPath: file) : Library.base.appending(path: file) }

    func preview(size: CGFloat) -> CTFont? {
        guard let descriptors = CTFontManagerCreateFontDescriptorsFromURL(url as CFURL) as? [CTFontDescriptor],
            descriptors.indices.contains(Int(index))
        else { return nil }
        return CTFontCreateWithFontDescriptor(descriptors[Int(index)], size, nil)
    }

    static func read(_ url: URL, file: String, requireCJK: Bool = false) throws -> [Self] {
        let size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        guard size > 0, size <= 32 << 20 else { throw PlayerError("Font must be at most 32 MiB.") }
        guard let descriptors = CTFontManagerCreateFontDescriptorsFromURL(url as CFURL) as? [CTFontDescriptor] else {
            throw PlayerError("Invalid OpenType font.")
        }
        return descriptors.enumerated().compactMap { index, desc in
            let font = CTFontCreateWithFontDescriptor(desc, 16, nil)
            let set = CTFontCopyCharacterSet(font)
            if requireCJK && ![0x4e2d, 0x3042].allSatisfy({ CFCharacterSetIsCharacterMember(set, UniChar($0)) }) {
                return nil
            }
            return Self(file: file, index: UInt32(index), name: CTFontCopyFullName(font) as String,
                        bytes: size, family: CTFontCopyFamilyName(font) as String)
        }
    }
}

func scoped<T>(_ url: URL, _ action: () throws -> T) rethrows -> T {
    let access = url.startAccessingSecurityScopedResource()
    defer { if access { url.stopAccessingSecurityScopedResource() } }
    return try action()
}

@Observable final class Library {
    struct Catalog: Codable, Equatable {
        var games = [Game]()
        var fonts = [FontFace]()
        var defaults = Options()
    }
    static let base = URL.applicationSupportDirectory.appending(path: "org.cmvs.player")
    static var userFiles: URL {
        #if os(macOS)
            base
        #else
            URL.documentsDirectory
        #endif
    }
    private let directory: URL
    private let legacyDirectory: URL?
    private var index: URL { directory.appending(path: "library.json") }
    var data = Catalog()
    var systemFonts = [FontFace]()
    var fonts: [FontFace] { data.fonts + systemFonts }
    var playbackRevision = [UUID: Int]()
    var defaultFont: FontFace? {
        data.defaults.font.isEmpty ? fonts.first : fonts.first { $0.id == data.defaults.font }
    }
    struct Issue {
        let title: String
        let message: String
        init(_ title: String.LocalizationValue, _ message: String) {
            self.title = String(localized: title)
            self.message = message
        }
    }
    var error: Issue?
    private(set) var loadError: String?
    var writable: Bool { loadError == nil }

    convenience init() {
        #if os(iOS)
            self.init(directory: Self.base, legacyDirectory: Self.userFiles)
        #else
            self.init(directory: Self.base)
        #endif
    }

    init(directory: URL, legacyDirectory: URL? = nil) {
        self.directory = directory
        self.legacyDirectory = legacyDirectory
        reload()
    }

    func reload() {
        do {
            let fm = FileManager.default
            try fm.createDirectory(at: directory, withIntermediateDirectories: true)
            if let legacyDirectory { try Self.migrateCatalog(from: legacyDirectory, to: directory) }
            data = fm.fileExists(atPath: index.path)
                ? try JSONDecoder().decode(Catalog.self, from: Data(contentsOf: index)) : Catalog()
            loadError = nil
        } catch {
            loadError = error.localizedDescription
        }
    }
    // Move the index last so an interrupted upgrade can resume on the next launch.
    static func migrateCatalog(from documents: URL, to support: URL) throws {
        let fm = FileManager.default
        let index = documents.appending(path: "library.json")
        let destination = support.appending(path: "library.json")
        guard !fm.fileExists(atPath: destination.path), fm.fileExists(atPath: index.path) else { return }
        let catalog = try JSONDecoder().decode(Catalog.self, from: Data(contentsOf: index))
        var paths = Set<String>()
        for game in catalog.games {
            paths.insert("\(game.id).png")
            if game.bookmark == nil { paths.insert("Games/\(game.id)") }
        }
        for face in catalog.fonts {
            let name = (face.file as NSString).lastPathComponent
            if face.file == "Fonts/" + name, ![".", "..", ""].contains(name) { paths.insert(face.file) }
        }
        for path in paths.sorted() {
            let source = documents.appending(path: path)
            guard fm.fileExists(atPath: source.path) else { continue }
            let target = support.appending(path: path)
            try fm.createDirectory(at: target.deletingLastPathComponent(), withIntermediateDirectories: true)
            try fm.moveItem(at: source, to: target)
        }
        for name in ["Games", "Fonts"] {
            // rmdir leaves directories containing user files intact.
            _ = rmdir(documents.appending(path: name).path)
        }
        try fm.moveItem(at: index, to: destination)
    }

    func persist() {
        guard writable else { return }
        do { try JSONEncoder().encode(data).write(to: index, options: .atomic) } catch {
            self.error = Issue("Couldn’t Save Library", error.localizedDescription)
        }
    }
    func font(for game: Game) throws -> FontFace {
        let face = game.options.font.isEmpty ? defaultFont : fonts.first { $0.id == game.options.font }
        guard let face else {
            throw PlayerError("Select a font in Fonts before playing.")
        }
        return face
    }

    static func inspect(_ source: URL, options: Options) throws -> GameInfo {
        let api = Bridge()
        try api.check(cmvs_apple_inspect_game(api.handle, source.path, options.profile, options.encoding))
        return try JSONDecoder().decode(GameInfo.self, from: Data(api.text.utf8))
    }

    private func removeIfPresent(_ url: URL) throws {
        do { try FileManager.default.removeItem(at: url) }
        catch let error as CocoaError where error.code == .fileNoSuchFile { }
    }

    func remove(_ game: Game) throws {
        try removeIfPresent(game.thumbnail)
        if game.bookmark == nil { try removeIfPresent(try game.resolve()) }
        data.games.removeAll { $0.id == game.id }
        playbackRevision.removeValue(forKey: game.id)
    }

    func removeFont(_ face: FontFace) throws {
        let directory = Self.base.appending(path: "Fonts", directoryHint: .isDirectory)
        guard data.fonts.contains(face), face.file == "Fonts/" + face.url.lastPathComponent,
            face.url.standardizedFileURL.deletingLastPathComponent().path == directory.standardizedFileURL.path
        else { return }
        try removeIfPresent(face.url)
        let ids = Set(data.fonts.filter { $0.file == face.file }.map(\.id))
        data.fonts.removeAll { $0.file == face.file }
        if ids.contains(data.defaults.font) { data.defaults.font = "" }
        for i in data.games.indices where ids.contains(data.games[i].options.font) { data.games[i].options.font = "" }
    }

    static func add(_ source: URL, copy: Bool, options: Options, progress: Progress) throws -> Game {
        let id = UUID()
        let destination = base.appending(path: "Games/\(id)")
        return try scoped(source) {
            do {
                let info = try inspect(source, options: options)
                if progress.isCancelled { throw CancellationError() }
                if copy { try copyGame(source, destination, progress) }
                let bookmark =
                    copy
                    ? nil
                    : try source.bookmarkData(
                        options: .minimalBookmark, includingResourceValuesForKeys: nil, relativeTo: nil)
                let game = Game(
                    id: id, name: source.lastPathComponent, info: info, bookmark: bookmark, options: options)
                #if os(iOS)
                    try Game.migrateSaves(source: source, destination: game.saves)
                #endif
                return game
            } catch {
                if copy { try? FileManager.default.removeItem(at: destination) }
                throw error
            }
        }
    }

    static func copyGame(_ source: URL, _ destination: URL, _ progress: Progress) throws {
        let fm = FileManager.default
        guard !destination.standardizedFileURL.path.hasPrefix(source.resolvingSymlinksInPath().path + "/") else {
            throw PlayerError("Choose the individual game folder.")
        }
        let keys: Set<URLResourceKey> = [.isDirectoryKey, .isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey]
        var items = [(URL, URL, Bool)]()
        var bytes: Int64 = 0
        func scan(_ from: URL, _ to: URL, _ depth: Int) throws {
            guard depth <= 32 else { throw PlayerError("Too many nested directories.") }
            for item in try fm.contentsOfDirectory(at: from, includingPropertiesForKeys: Array(keys)) {
                if progress.isCancelled { throw CancellationError() }
                let v = try item.resourceValues(forKeys: keys)
                guard v.isSymbolicLink != true, v.isRegularFile == true || v.isDirectory == true,
                    items.count < 100000
                else { throw PlayerError("Unsupported entry in game directory.") }
                let target = to.appending(path: item.lastPathComponent)
                items.append((item, target, v.isDirectory == true))
                if v.isDirectory == true { try scan(item, target, depth + 1) } else { bytes += Int64(v.fileSize ?? 0) }
            }
        }
        try scan(source, destination, 0)
        progress.totalUnitCount = max(bytes, 1)
        try fm.createDirectory(at: destination, withIntermediateDirectories: true)
        for (from, to, dir) in items {
            if progress.isCancelled { throw CancellationError() }
            if dir {
                try fm.createDirectory(at: to, withIntermediateDirectories: false)
                continue
            }
            guard !fm.fileExists(atPath: to.path), fm.createFile(atPath: to.path, contents: nil) else {
                throw PlayerError("Cannot create game file.")
            }
            let input = try FileHandle(forReadingFrom: from)
            let output = try FileHandle(forWritingTo: to)
            defer {
                try? input.close()
                try? output.close()
            }
            while let chunk = try input.read(upToCount: 1 << 20), !chunk.isEmpty {
                if progress.isCancelled { throw CancellationError() }
                try output.write(contentsOf: chunk)
                progress.completedUnitCount += Int64(chunk.count)
            }
        }
    }

    static func importFont(_ source: URL) throws -> [FontFace] {
        try scoped(source) {
            let file = "Fonts/\(UUID())-\(source.lastPathComponent)"
            let faces = try FontFace.read(source, file: file)
            guard let first = faces.first else { throw PlayerError("No readable font faces.") }
            let api = Bridge()
            for face in faces { try api.check(cmvs_apple_validate_font(api.handle, source.path, face.index)) }
            try FileManager.default.createDirectory(
                at: first.url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try FileManager.default.copyItem(at: source, to: first.url)
            return faces
        }
    }

    #if os(macOS)
        static func scanFonts() -> [FontFace] {
            let directories = [
                "/System/Library/Fonts", "/System/Library/Fonts/Supplemental", "/Library/Fonts",
                FileManager.default.homeDirectoryForCurrentUser.appending(path: "Library/Fonts").path,
            ]
            return directories.flatMap { dir in
                ((try? FileManager.default.contentsOfDirectory(
                    at: URL(fileURLWithPath: dir), includingPropertiesForKeys: nil)) ?? [])
                    .filter { ["ttf", "ttc", "otf"].contains($0.pathExtension.lowercased()) }
                    .flatMap { (try? FontFace.read($0, file: $0.path, requireCJK: true)) ?? [] }
            }.sorted { $0.name < $1.name }
        }
    #endif
}
