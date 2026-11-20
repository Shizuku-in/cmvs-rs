import CmvsAppleC
import SwiftUI
import UniformTypeIdentifiers

struct SaveRow: View {
    var game: Game
    var library: Library
    @State private var exported: URL?
    @State private var selected: URL?
    @State private var picking = false
    @State private var confirm = false
    @State private var busy = false
    @State private var summary = ""
    var body: some View {
        VStack(alignment: .leading) {
            Text(game.name).font(.headline)
            Text(summary).font(.caption)
            HStack {
                if let exported { ShareLink("Share ZIP", item: exported) }
                Button("Export ZIP") { backup(nil) }
                Button("Restore ZIP") { picking = true }
                #if os(macOS)
                    Button("Show in Finder") {
                        if let dir = try? directory() { NSWorkspace.shared.activateFileViewerSelecting([dir]) }
                    }
                #endif
            }.disabled(busy)
        }
        .task(id: game.options.game_savedata) { refresh() }
        .fileImporter(isPresented: $picking, allowedContentTypes: [.zip]) { result in
            if case .success(let url) = result {
                selected = url
                confirm = true
            }
        }
        .confirmationDialog("Replace matching save files?", isPresented: $confirm) {
            Button("Restore", role: .destructive) { if let selected { backup(selected) } }
        }
    }
    func directory() throws -> URL {
        if game.options.game_savedata {
            let dir = try game.resolve().appending(path: "savedata")
            if FileManager.default.isWritableFile(atPath: dir.path) { return dir }
        }
        return game.saves
    }
    func refresh() {
        do {
            let dir = try directory()
            let files =
                (try? FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: [.fileSizeKey]))
                ?? []
            let systemName = game.info.family == "CVNS" ? "save000.dat" : "system.dat"
            let system = files.contains { $0.lastPathComponent == systemName }
            let slots = files.filter {
                $0.lastPathComponent.range(of: #"^save[0-9]{3}\.dat$"#, options: .regularExpression) != nil
                    && $0.lastPathComponent != systemName
            }.count
            let faces = (try? FileManager.default.contentsOfDirectory(at: dir.appending(path: "font-faces"), includingPropertiesForKeys: [.fileSizeKey])) ?? []
            let bytes = (files + faces).reduce(0) { $0 + ((try? $1.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0) }
            summary =
                "\(system ? 1 : 0) system · \(slots) slots · \(ByteCountFormatter.string(fromByteCount:Int64(bytes),countStyle:.file))"
        } catch { summary = error.localizedDescription }
    }
    func backup(_ source: URL?) {
        busy = true
        Task {
            defer { busy = false }
            do {
                let game = game
                let output = try await Task.detached {
                    let root = try game.resolve()
                    let url = source ?? FileManager.default.temporaryDirectory.appending(path: "Saves-\(UUID()).zip")
                    try scoped(root) {
                        try scoped(url) {
                            let api = Bridge()
                            try api.check(
                                cmvs_apple_backup(
                                    api.handle, root.path, game.saves.path, game.options.game_savedata, url.path,
                                    source != nil))
                        }
                    }
                    return url
                }.value
                exported = source == nil ? output : nil
                refresh()
            } catch { library.error = error.localizedDescription }
        }
    }
}
