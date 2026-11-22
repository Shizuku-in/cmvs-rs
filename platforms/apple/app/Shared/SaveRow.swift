import CmvsAppleC
import CoreTransferable
import SwiftUI
import UniformTypeIdentifiers

final class SaveArchive: Transferable {
    let url: URL
    init(game: Game) throws {
        let directory = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        url = directory.appending(path: game.filename + ".zip")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
    }
    deinit { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }
    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(exportedContentType: .zip) { SentTransferredFile($0.url) }
            // SwiftUI can require a suggested name even when fileExporter has a default filename.
            .suggestedFileName { $0.url.lastPathComponent }
    }
}

private struct SaveSummary {
    var directory: URL?
    var system = 0
    var slots = 0
    var bytes: Int64 = 0

    static func read(_ game: Game) throws -> Self {
        guard game.options.game_savedata else { return try read(game.saves, family: game.info.family) }
        let root = try game.resolve()
        return try scoped(root) {
            let local = root.appending(path: "savedata")
            let dir = FileManager.default.isWritableFile(atPath: local.path) ? local : game.saves
            return try read(dir, family: game.info.family)
        }
    }

    private static func read(_ dir: URL, family: String) throws -> Self {
        let fm = FileManager.default
        var isDirectory: ObjCBool = false
        guard fm.fileExists(atPath: dir.path, isDirectory: &isDirectory), isDirectory.boolValue else { return Self() }
        let keys: Set<URLResourceKey> = [.isDirectoryKey, .isRegularFileKey, .fileSizeKey]
        let files = try fm.contentsOfDirectory(at: dir, includingPropertiesForKeys: Array(keys))
        var summary = Self(directory: dir)
        let system = family == "CVNS" ? "save000.dat" : "system.dat"
        for file in files {
            let name = file.lastPathComponent
            let values = try file.resourceValues(forKeys: keys)
            if name == "font-faces", values.isDirectory == true {
                for face in try fm.contentsOfDirectory(at: file, includingPropertiesForKeys: Array(keys)) {
                    let v = try face.resourceValues(forKeys: keys)
                    if v.isRegularFile == true { summary.bytes += Int64(v.fileSize ?? 0) }
                }
            }
            guard values.isRegularFile == true, !name.hasPrefix(".") else { continue }
            if name == system {
                summary.system = 1
            } else if name.range(of: #"^save[0-9]{3}\.dat$"#, options: .regularExpression) != nil {
                summary.slots += 1
            }
            summary.bytes += Int64(values.fileSize ?? 0)
        }
        return summary
    }
}

struct SaveRow: View {
    var game: Game
    var library: Library
    @State private var archive: SaveArchive?
    @State private var exporting = false
    @State private var selected: URL?
    @State private var picking = false
    @State private var confirm = false
    @State private var busy = false
    @State private var feedback: String?
    @State private var summary = SaveSummary()
    @State private var readError: String?
    private struct RefreshKey: Equatable {
        let game: Game
        let revision: Int?
    }
    var body: some View {
        VStack(alignment: .leading) {
            Text(game.name).font(.headline)
            if let readError {
                Text(readError).font(.caption)
            } else {
                let bytes = ByteCountFormatter.string(fromByteCount: summary.bytes, countStyle: .file)
                Text("\(summary.system) system · ^[\(summary.slots) slot](inflect: true) · \(bytes)")
                    .font(.caption)
            }
            ViewThatFits(in: .horizontal) {
                HStack { actions }.fixedSize(horizontal: true, vertical: false)
                VStack(alignment: .leading) { actions }
            }.buttonStyle(.borderless).disabled(busy || exporting)
            if busy { ProgressView().controlSize(.small).accessibilityLabel("Processing Saves") }
            if let feedback {
                Label(feedback, systemImage: "checkmark.circle.fill")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .task(id: RefreshKey(game: game, revision: library.playbackRevision[game.id])) { await refresh() }
        .fileExporter(isPresented: $exporting, item: archive, contentTypes: [.zip], defaultFilename: game.filename) { result in
            archive = nil
            switch result {
            case .success(let url): feedback = String(localized: "Exported \(url.lastPathComponent)")
            case .failure(let error): library.error = .init("Couldn’t Export Saves", error.localizedDescription)
            }
        } onCancellation: { archive = nil }
        .fileImporter(isPresented: $picking, allowedContentTypes: [.zip]) { result in
            switch result {
            case .success(let url):
                selected = url
                confirm = true
            case .failure(let error):
                if (error as NSError).code != NSUserCancelledError { library.error = .init("Couldn’t Restore Saves", error.localizedDescription) }
            }
        }
        .confirmationDialog("Restore Saves for \(game.name)?", isPresented: $confirm, titleVisibility: .visible) {
            Button("Restore", role: .destructive) { if let selected { backup(selected) } }
        } message: { Text("Matching save files will be replaced.") }
    }

    @ViewBuilder private var actions: some View {
        Group {
            Button { backup(nil) } label: { Label("Export ZIP…", systemImage: "square.and.arrow.up").controlHitArea() }
                .accessibilityLabel("Export Saves for \(game.name)").disabled(summary.system + summary.slots == 0)
            Button { picking = true } label: { Label("Restore ZIP…", systemImage: "square.and.arrow.down").controlHitArea() }
                .accessibilityLabel("Restore Saves for \(game.name)")
            #if os(macOS)
                Button("Show in Finder", systemImage: "folder") {
                    if let dir = summary.directory { NSWorkspace.shared.activateFileViewerSelecting([dir]) }
                }.disabled(summary.directory == nil)
            #endif
        }.labelStyle(.titleAndIcon)
    }

    private func refresh() async {
        let game = game
        do {
            let result = try await Task.detached { try SaveSummary.read(game) }.value
            guard !Task.isCancelled else { return }
            summary = result
            readError = nil
        } catch {
            guard !Task.isCancelled else { return }
            summary = SaveSummary()
            readError = error.localizedDescription
        }
    }

    private func backup(_ source: URL?) {
        busy = true
        feedback = nil
        Task {
            defer { busy = false }
            do {
                let game = game
                let output = try await Task.detached {
                    let api = Bridge()
                    if let source {
                        let root = try game.resolve()
                        try scoped(root) {
                            try scoped(source) {
                                try api.check(cmvs_apple_restore_saves(api.handle, root.path, game.options.profile,
                                    game.info.encoding, game.saves.path, game.options.game_savedata, source.path))
                            }
                        }
                        return nil as SaveArchive?
                    }
                    let archive = try SaveArchive(game: game)
                    let identity = game.info.backupIdentity ?? game.info.game ?? ""
                    if game.options.game_savedata {
                        let root = try game.resolve()
                        try scoped(root) {
                            try api.check(cmvs_apple_export_saves(api.handle, root.path, game.saves.path, archive.url.path, identity))
                        }
                    } else {
                        try api.check(cmvs_apple_export_saves(api.handle, nil, game.saves.path, archive.url.path, identity))
                    }
                    return archive
                }.value
                if let output {
                    archive = output
                    exporting = true
                } else {
                    feedback = String(localized: "Saves restored.")
                    await refresh()
                }
            } catch { library.error = .init(source == nil ? "Couldn’t Export Saves" : "Couldn’t Restore Saves", error.localizedDescription) }
        }
    }
}
