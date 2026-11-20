import CmvsAppleC
import ImageIO
import Metal
import SwiftUI
import UniformTypeIdentifiers

@main struct CmvsApp: App {
    #if os(iOS)
        @UIApplicationDelegateAdaptor(OrientationDelegate.self) private var orientation
    #endif
    @State private var library = Library()
    @State private var player: Playback?
    var body: some Scene {
        #if os(macOS)
            Window("CMVS", id: "library") {
                Launcher(library: library, player: $player)
                    .frame(minWidth: 660, minHeight: 440)
            }
            .commands {
                CommandGroup(after: .newItem) {
                    Button("Add game") { NotificationCenter.default.post(name: .addGame, object: nil) }
                        .keyboardShortcut("o").disabled(player != nil)
                }
                CommandMenu("Game") {
                    Button("Full screen") { NSApp.keyWindow?.toggleFullScreen(nil) }.keyboardShortcut("f")
                    Button("Screenshot") { player?.capture() }.keyboardShortcut("s").disabled(player?.ready != true)
                }
            }
            Window("Player", id: "player") {
                if let player { PlayerWindow(player: player, current: $player) }
            }
            .defaultSize(width: 960, height: 540)
        #else
            WindowGroup { Launcher(library: library, player: $player) }
        #endif
    }
}

extension Notification.Name { static let addGame = Notification.Name("CMVS.AddGame") }

#if os(macOS)
    private struct PlayerWindow: View {
        let player: Playback
        @Binding var current: Playback?
        @Environment(\.dismissWindow) var dismiss
        var body: some View {
            PlayerScreen(player: player)
                .onAppear { player.quit = { dismiss(id: "player") } }
                .onDisappear {
                    player.close()
                    current = nil
                }
        }
    }
#endif

private enum SectionName: String, CaseIterable, Identifiable {
    case games = "Library"
    case fonts = "Fonts"
    case saves = "Saves"
    case settings = "Settings"
    var id: Self { self }
    var icon: String {
        switch self {
        case .games: "books.vertical"
        case .fonts: "textformat"
        case .saves: "archivebox"
        case .settings: "gearshape"
        }
    }
}

private struct Launcher: View {
    @Bindable var library: Library
    @Binding var player: Playback?
    #if os(iOS)
        @Environment(\.horizontalSizeClass) var horizontalSize
        var compact: Bool { horizontalSize == .compact }
    #else
        let compact = false
    #endif
    #if os(macOS)
        @Environment(\.openWindow) var openWindow
    #endif
    @State private var section: SectionName? = .games
    @State private var picking = false
    @State private var pickFont = false
    @State private var copyGame = false
    @State private var progress: Progress?
    @State private var editing: UUID?
    @State private var grid = true
    @State private var smokeStarted = false

    var body: some View {
        Group {
            if compact {
                TabView(selection: $section) {
                    ForEach(SectionName.allCases) { item in
                        NavigationStack { content(item) }.tabItem {
                            Label(LocalizedStringKey(item.rawValue), systemImage: item.icon)
                        }.tag(Optional(item))
                    }
                }
            } else {
                NavigationSplitView {
                    List(SectionName.allCases, selection: $section) { item in
                        Label(LocalizedStringKey(item.rawValue), systemImage: item.icon).tag(item)
                    }.navigationTitle("CMVS")
                } detail: {
                    content(section ?? .games)
                }
            }
        }
        .onChange(of: library.data) { _, _ in library.persist() }
        .onReceive(NotificationCenter.default.publisher(for: .addGame)) { _ in choose(false, false) }
        .fileImporter(isPresented: $picking, allowedContentTypes: pickFont ? [.font] : [.folder]) { result in
            do {
                let url = try result.get()
                if pickFont { addFont(url) } else { addGame(url, copyGame) }
            } catch {
                if (error as NSError).code != NSUserCancelledError { library.error = error.localizedDescription }
            }
        }
        .sheet(isPresented: Binding(get: { editing != nil }, set: { if !$0 { editing = nil } })) {
            if let i = library.data.games.firstIndex(where: { $0.id == editing }) {
                NavigationStack {
                    OptionsForm(options: $library.data.games[i].options, fonts: library.fonts, global: false)
                        .navigationTitle(library.data.games[i].name)
                        .toolbar { Button("Done") { editing = nil } }
                }.frame(minWidth: 300, minHeight: 350)
            }
        }
        .alert("CMVS", isPresented: Binding(get: { library.error != nil }, set: { if !$0 { library.error = nil } })) {
            Button("OK") { library.error = nil }
        } message: {
            Text(library.error ?? "")
        }
        #if os(iOS)
            .fullScreenCover(item: $player) { session in
                PlayerScreen(player: session).persistentSystemOverlays(.hidden)
                .onAppear {
                    OrientationDelegate.playing(true)
                    session.quit = { player = nil }
                }
                .onDisappear {
                    session.close()
                    OrientationDelegate.playing(false)
                }
            }
        #endif
        .task {
            #if os(macOS)
                library.systemFonts = await Task.detached { Library.scanFonts() }.value
            #endif
            let env = ProcessInfo.processInfo.environment
            if let path = env["CMVS_GAME_DIR"], !smokeStarted {
                smokeStarted = true
                do {
                    var options = library.data.defaults
                    options.profile = env["CMVS_PROFILE"] ?? options.profile
                    #if targetEnvironment(simulator)
                        if options.metal && MTLCreateSystemDefaultDevice() == nil {
                            options.metal = false
                        }
                    #endif
                    NSLog("CMVS_SMOKE renderer=%@", options.metal ? "Metal" : "CPU")
                    if let path = env["CMVS_FONT"] {
                        let fonts = try FontFace.read(URL(fileURLWithPath: path), file: path)
                        library.systemFonts += fonts
                        options.font = fonts.first?.id ?? ""
                    }
                    let game = try await Task.detached {
                        try Library.add(
                            URL(fileURLWithPath: path), copy: false, options: options,
                            progress: Progress(totalUnitCount: -1))
                    }.value
                    play(game)
                } catch {
                    NSLog("CMVS_SMOKE launch failed: %@", error.localizedDescription)
                    library.error = error.localizedDescription
                }
            }
        }
    }

    @ViewBuilder func content(_ item: SectionName) -> some View {
        Group {
            switch item {
            case .games:
                VStack {
                    if let progress {
                        HStack {
                            ProgressView(progress)
                            Button("Cancel") { progress.cancel() }
                        }.padding()
                    }
                    if library.data.games.isEmpty {
                        ContentUnavailableView(
                            "Game library", systemImage: "books.vertical",
                            description: Text("Add a folder containing data/pack."))
                    } else {
                        ScrollView {
                            LazyVGrid(
                                columns: grid ? [GridItem(.adaptive(minimum: 190))] : [GridItem(.flexible())],
                                alignment: .leading
                            ) {
                                ForEach(library.data.games) { game in card(game) }
                            }.padding()
                        }
                    }
                }
                .dropDestination(for: URL.self) { urls, _ in
                    guard let url = urls.first, progress == nil else { return false }
                    addGame(url, false)
                    return true
                }
                .toolbar {
                    Button {
                        grid.toggle()
                    } label: {
                        Label("Change layout", systemImage: grid ? "list.bullet" : "square.grid.2x2")
                    }
                    #if os(iOS)
                        Menu {
                            Button("Reference folder") { choose(false, false) }
                            Button("Copy into library") { choose(false, true) }
                        } label: {
                            Label("Add game", systemImage: "plus")
                        }.disabled(progress != nil)
                    #else
                        Button {
                            choose(false, false)
                        } label: {
                            Label("Add game", systemImage: "plus")
                        }.disabled(progress != nil)
                    #endif
                }
            case .fonts:
                List(library.fonts) { face in
                    VStack(alignment: .leading) {
                        Text(face.name)
                        if let preview = face.preview { Text(verbatim: "日本語 · 简体中文").font(Font(preview)) }
                        Text(
                            "\(ByteCountFormatter.string(fromByteCount: Int64(face.bytes),countStyle: .file)) · #\(face.index)"
                        ).foregroundStyle(.secondary)
                    }
                    Button(library.data.defaults.font == face.id ? "Default" : "Set default") {
                        library.data.defaults.font = face.id
                    }
                }
                .toolbar { Button("Import font") { choose(true, false) } }
            case .settings:
                OptionsForm(options: $library.data.defaults, fonts: library.fonts, global: true)
            case .saves:
                List(library.data.games) { game in SaveRow(game: game, library: library).disabled(player != nil) }
            }
        }.navigationTitle(LocalizedStringKey(item.rawValue))
    }
    func card(_ game: Game) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            let thumbnail = Library.base.appending(path: "\(game.id).png")
            if let source = CGImageSourceCreateWithURL(thumbnail as CFURL, nil),
                let image = CGImageSourceCreateThumbnailAtIndex(
                    source, 0,
                    [kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceThumbnailMaxPixelSize: 360]
                        as CFDictionary)
            {
                Image(decorative: image, scale: 1).resizable().aspectRatio(contentMode: .fit)
            }
            Text(game.name).font(.headline)
            Text("\(game.info.family) · \(game.info.encoding)").foregroundStyle(.secondary)
            if let last = game.lastPlayed { Text(last, format: .dateTime.year().month().day()).font(.caption) }
            HStack {
                Button("Play") { play(game) }.disabled(player != nil)
                Button("Configure") { editing = game.id }
            }
        }.padding()
            .contextMenu {
                Button("Configure") { editing = game.id }
                #if os(macOS)
                    Button("Show in Finder") {
                        if let url = try? game.resolve() { NSWorkspace.shared.activateFileViewerSelecting([url]) }
                    }
                #endif
                Button("Remove from library", role: .destructive) { library.data.games.removeAll { $0.id == game.id } }
            }
    }
    func choose(_ font: Bool, _ copy: Bool) {
        pickFont = font
        copyGame = copy
        picking = true
    }
    func addFont(_ url: URL) {
        Task {
            do {
                let faces = try await Task.detached { try Library.importFont(url) }.value
                library.data.fonts += faces
            } catch { library.error = error.localizedDescription }
        }
    }
    func addGame(_ url: URL, _ copy: Bool) {
        guard progress == nil else { return }
        let job = Progress(totalUnitCount: -1)
        progress = job
        var options = library.data.defaults
        options.font = ""
        Task {
            defer { progress = nil }
            do {
                let game = try await Task.detached { try Library.add(url, copy: copy, options: options, progress: job) }
                    .value
                if !job.isCancelled { library.data.games.append(game) }
            } catch { if !job.isCancelled { library.error = error.localizedDescription } }
        }
    }
    func play(_ game: Game) {
        do {
            var game = game
            var stale = false
            let root = try game.resolve(stale: &stale)
            if stale {
                game.bookmark = try scoped(root) {
                    try root.bookmarkData(
                        options: .minimalBookmark, includingResourceValuesForKeys: nil, relativeTo: nil)
                }
            }
            let font = try library.font(for: game)
            let session = Playback(game, font, library.fonts)
            session.closeError = { library.error = $0 }
            if let i = library.data.games.firstIndex(where: { $0.id == game.id }) {
                game.lastPlayed = Date()
                library.data.games[i] = game
            }
            player = session
            #if os(macOS)
                openWindow(id: "player")
            #endif
        } catch {
            if smokeStarted { NSLog("CMVS_SMOKE playback setup failed: %@", error.localizedDescription) }
            library.error = error.localizedDescription
        }
    }
}

private struct OptionsForm: View {
    @Binding var options: Options
    var fonts: [FontFace]
    let global: Bool
    private static let profiles = Bridge.profiles()
    var body: some View {
        Form {
            Picker("Text encoding", selection: $options.encoding) {
                ForEach(["auto", "shift-jis", "gbk", "utf-8"], id: \.self) { Text($0).tag($0) }
            }
            Picker("Font", selection: $options.font) {
                Text(global ? "Automatic" : "Global default").tag("")
                ForEach(fonts) { Text($0.name).tag($0.id) }
            }
            Picker("Renderer", selection: $options.metal) {
                Text("Metal").tag(true)
                Text("CPU").tag(false)
            }
            Picker("Scaling", selection: $options.aspect) {
                Text("Game preference").tag(nil as Int?)
                Text("Original size").tag(Optional(0))
                Text("Fit").tag(Optional(1))
                Text("Fill").tag(Optional(2))
                Text("Stretch").tag(Optional(3))
            }
            Toggle("Native audio", isOn: $options.native_audio)
            #if os(macOS)
                Toggle("Use game savedata folder", isOn: $options.game_savedata)
            #else
                Toggle("Multi-touch gestures", isOn: $options.gestures)
            #endif
            Section("Diagnostics") {
                Picker("Engine profile", selection: $options.profile) {
                    Text("Automatic").tag("")
                    ForEach(Self.profiles, id: \.self) { Text($0).tag($0) }
                }
                Toggle("Skip auto-resume", isOn: $options.skip_auto_resume)
                Toggle("Continue without snapshot", isOn: $options.continue_without_snapshot)
                Toggle("Block presumed commands", isOn: $options.block_presumed)
                Toggle("Block unreviewed commands", isOn: $options.block_unreviewed)
            }
        }.formStyle(.grouped)
    }
}
