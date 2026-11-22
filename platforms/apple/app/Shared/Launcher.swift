import CmvsAppleC
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
        Group {
            #if os(macOS)
                Window("CMVS", id: "library") {
                    Launcher(library: library, player: $player)
                        .frame(minWidth: 660, minHeight: 440)
                }
                Window("Player", id: "player") { PlayerWindow(current: $player) }
                Settings {
                    OptionsForm(options: $library.data.defaults, fonts: library.fonts, global: true)
                        .disabled(!library.writable)
                        .frame(minWidth: 480, minHeight: 500)
                }
            #else
                WindowGroup { Launcher(library: library, player: $player) }
            #endif
        }
        .commands { LauncherCommands(player: player) }
        .onChange(of: library.data) { library.persist() }
    }
}

private struct LauncherCommands: Commands {
    let player: Playback?
    @AppStorage("showPerformance") private var showPerformance = false
    @FocusedValue(\.addGame) private var addGame
    @FocusedValue(\.gamePlayer) private var gamePlayer

    var body: some Commands {
        CommandGroup(after: .newItem) {
            Button("Add Game…") { addGame?() }
                .keyboardShortcut("o").disabled(addGame == nil || player != nil)
        }
        CommandMenu("Game") {
            Button("Screenshot") { gamePlayer?.capture() }.disabled(gamePlayer?.canCapture != true)
            Divider()
            Toggle("Shift Fast-Forward", isOn: gamePlayer?.fastForwardBinding(.shift) ?? .constant(false))
                .disabled(gamePlayer?.canFastForward != true)
            Toggle("Ctrl Fast-Forward", isOn: gamePlayer?.fastForwardBinding(.control) ?? .constant(false))
                .disabled(gamePlayer?.canFastForward != true)
            Button("Hide Text") { gamePlayer?.pulse(26) }.disabled(gamePlayer?.canFastForward != true)
            Toggle("Show Performance", isOn: $showPerformance)
        }
    }
}

private struct AddGameKey: FocusedValueKey {
    typealias Value = () -> Void
}

private struct GamePlayerKey: FocusedValueKey {
    typealias Value = Playback
}

extension FocusedValues {
    var gamePlayer: Playback? {
        get { self[GamePlayerKey.self] }
        set { self[GamePlayerKey.self] = newValue }
    }

    var addGame: (() -> Void)? {
        get { self[AddGameKey.self] }
        set { self[AddGameKey.self] = newValue }
    }
}

extension View {
    func controlHitArea() -> some View {
        #if os(iOS)
            frame(minWidth: 44, minHeight: 44).contentShape(Rectangle())
        #else
            frame(minWidth: 20, minHeight: 20).contentShape(Rectangle())
        #endif
    }
}

#if os(macOS)
    private struct PlayerWindow: View {
        @Binding var current: Playback?
        @Environment(\.dismissWindow) private var dismiss
        var body: some View {
            if let player = current {
                PlayerScreen(player: player)
                    .frame(minWidth: 320, minHeight: 240)
                    .onAppear { player.quit = { dismiss(id: "player") } }
                    .onDisappear {
                        player.close()
                        if current === player { current = nil }
                    }
            } else {
                Color.clear.task { if current == nil { dismiss(id: "player") } }
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
        case .fonts: "f.cursive"
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
    @Environment(\.colorSchemeContrast) private var contrast
    @Environment(\.dynamicTypeSize) private var typeSize
    @ScaledMetric(relativeTo: .body) private var cardWidth: CGFloat = 190
    @State private var section: SectionName? = .games
    @State private var picking = false
    @State private var pickFont = false
    @State private var copyGame = false
    @State private var progress: Progress?
    @State private var editing: Game?
    @State private var removing: Game?
    @State private var targeted = false
    @State private var grid = true
    @State private var showingGuide = false
    @State private var smokeStarted = false

    var body: some View {
        Group {
            if let reason = library.loadError {
                ContentUnavailableView {
                    Label("Couldn’t Open Library", systemImage: "books.vertical")
                } description: {
                    Text(verbatim: reason).textSelection(.enabled)
                } actions: {
                    Button("Retry") { library.reload() }
                }
            } else if compact {
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
        .focusedSceneValue(\.addGame, canChooseGame ? { choose(false, false) } : nil)
        .fileImporter(isPresented: $picking, allowedContentTypes: pickFont ? [.font] : [.folder]) { result in
            do {
                let url = try result.get()
                if pickFont { addFont(url) } else { addGame(url, copyGame) }
            } catch {
                if (error as NSError).code != NSUserCancelledError {
                    library.error = .init(pickFont ? "Couldn’t Import Font" : "Couldn’t Add Game", error.localizedDescription)
                }
            }
        }
        .sheet(item: $editing) { game in GameConfiguration(game: game, library: library) }
        .sheet(isPresented: $showingGuide) { ControlsGuideView() }
        .alert(Text(verbatim: library.error?.title ?? ""), isPresented: Binding(
            get: { library.error != nil }, set: { if !$0 { library.error = nil } })
        ) {
            Button("OK") { library.error = nil }
        } message: {
            Text(verbatim: library.error?.message ?? "")
        }
        #if os(iOS)
            .fullScreenCover(item: $player) { session in
                PlayerScreen(player: session).persistentSystemOverlays(.hidden).statusBarHidden(true)
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
            library.systemFonts = await Task.detached { Library.scanFonts() }.value
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
                    library.error = .init("Couldn’t Open Game", error.localizedDescription)
                }
            }
        }
    }

    private var canChooseGame: Bool {
        library.writable && progress == nil && !picking && editing == nil
    }

    @ViewBuilder func content(_ item: SectionName) -> some View {
        Group {
            switch item {
            case .games:
                VStack {
                    if library.data.games.isEmpty {
                        ContentUnavailableView {
                            Label("Game Library", systemImage: "books.vertical")
                        } description: { Text("Add a folder containing data/pack.") } actions: { addGameButton }
                    } else {
                        ScrollView {
                            if grid {
                                LazyVGrid(columns: [GridItem(.adaptive(minimum: cardWidth))], alignment: .leading) {
                                    ForEach(library.data.games) { game in card(game) }
                                }.padding()
                            } else {
                                LazyVStack { ForEach(library.data.games) { game in card(game) } }.padding()
                            }
                        }
                    }
                }
                .dropDestination(for: URL.self) { urls, _ in
                    guard let url = urls.first, progress == nil else { return false }
                    addGame(url, false)
                    return true
                } isTargeted: { targeted = $0 }
                .overlay {
                    if targeted { RoundedRectangle(cornerRadius: 10).strokeBorder(.tint, lineWidth: 3).allowsHitTesting(false) }
                }
                .confirmationDialog("Remove from Library?", isPresented: Binding(
                    get: { removing != nil }, set: { if !$0 { removing = nil } }
                ), titleVisibility: .visible, presenting: removing) { game in
                    Button("Remove from Library", role: .destructive) {
                        do { try library.remove(game) } catch { library.error = .init("Couldn’t Remove Game", error.localizedDescription) }
                    }
                } message: { game in
                    if game.bookmark == nil {
                        Text("Delete the library copy of \(game.name) and its cover?")
                    } else {
                        Text("Remove \(game.name) and its cover? The referenced folder will be kept.")
                    }
                }
                .toolbar {
                    Button { showingGuide = true } label: {
                        Label("Controls Guide", systemImage: "questionmark.circle")
                    }
                    .help("Controls Guide")
                    Button {
                        grid.toggle()
                    } label: {
                        Label("Change Layout", systemImage: grid ? "list.bullet" : "square.grid.2x2")
                    }
                    addGameButton
                }
            case .fonts:
                FontList(library: library, playing: player != nil) { choose(true, false) }
            case .settings:
                OptionsForm(options: $library.data.defaults, fonts: library.fonts, global: true)
            case .saves:
                if library.data.games.isEmpty {
                    ContentUnavailableView(
                        "No Games in the Library", systemImage: "archivebox",
                        description: Text("Add a game to the library to manage its saves here."))
                } else {
                    List(library.data.games) { game in SaveRow(game: game, library: library).disabled(player != nil) }
                }
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            if let progress {
                HStack {
                    ImportProgress(progress: progress)
                    Spacer()
                    Button("Cancel") { progress.cancel() }
                }.padding().background(.bar)
            }
        }
        .navigationTitle(LocalizedStringKey(item.rawValue))
    }
    private var addGameButton: some View {
        Group {
            #if os(iOS)
                Menu {
                    Button("Reference Folder…") { choose(false, false) }
                    Button("Copy into Library…") { choose(false, true) }
                } label: { Label("Add Game…", systemImage: "plus") }
            #else
                Button("Add Game…", systemImage: "plus") { choose(false, false) }
            #endif
        }.disabled(progress != nil)
    }

    func card(_ game: Game) -> some View {
        let cover = GameCover(game: game, revision: library.playbackRevision[game.id, default: 0])
        let details = VStack(alignment: .leading, spacing: 4) {
            Text(game.name).font(.headline).lineLimit(typeSize.isAccessibilitySize ? nil : 2)
                .fixedSize(horizontal: false, vertical: true)
            Text(verbatim: "\(game.info.profile) · \(game.info.encodingLabel)").font(.caption).foregroundStyle(.secondary)
            if !grid && !typeSize.isAccessibilitySize, let last = game.lastPlayed {
                Text(last, format: .dateTime.year().month().day()).font(.caption).foregroundStyle(.secondary)
            }
        }
        #if os(iOS)
            let chipSize: CGFloat = 32
            let actionSpacing: CGFloat = 0
        #else
            let chipSize: CGFloat = 28
            let actionSpacing: CGFloat = 6
        #endif
        let configure = Button { editing = game } label: {
            Image(systemName: "gearshape")
                .frame(width: chipSize, height: chipSize)
                .background(.secondary.opacity(0.14), in: Circle())
                .controlHitArea()
        }
        .buttonStyle(.borderless)
        .foregroundStyle(.secondary)
        .accessibilityLabel("Configure \(game.name)")
        .help("Configure")
        .disabled(player?.game.id == game.id)
        let playButton = Button { play(game) } label: {
            Image(systemName: "play.fill")
                .frame(width: chipSize, height: chipSize)
                .background(Color.accentColor.opacity(0.16), in: Circle())
                .controlHitArea()
        }
        .buttonStyle(.borderless)
        .accessibilityLabel("Play \(game.name)")
        .help("Play")
        .disabled(player != nil)
        let actions = (typeSize.isAccessibilitySize ? AnyLayout(VStackLayout(alignment: .leading)) : AnyLayout(HStackLayout(spacing: actionSpacing))) {
            playButton
            configure
        }
        return Group {
            if grid || typeSize.isAccessibilitySize {
                VStack(alignment: .leading, spacing: 8) {
                    cover
                    details
                    if typeSize.isAccessibilitySize {
                        if let last = game.lastPlayed {
                            Text(last, format: .dateTime.year().month().day()).font(.caption).foregroundStyle(.secondary)
                        }
                        actions
                    } else {
                        HStack(alignment: .center, spacing: 8) {
                            if let last = game.lastPlayed {
                                Text(last, format: .dateTime.year().month().day())
                                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                            }
                            Spacer(minLength: 0)
                            actions
                        }
                    }
                }
            } else {
                HStack {
                    cover.frame(width: 76)
                    details
                    Spacer(minLength: 0)
                    actions
                }
            }
        }
        .padding(10).background(.secondary.opacity(0.06), in: RoundedRectangle(cornerRadius: 10))
        .overlay { RoundedRectangle(cornerRadius: 10).strokeBorder(.secondary.opacity(contrast == .increased ? 1 : 0.25)).allowsHitTesting(false) }
        .contentShape(Rectangle())
        #if os(macOS)
            .onTapGesture(count: 2) { play(game) }
        #endif
        .contextMenu {
            Button("Play") { play(game) }.disabled(player != nil)
            Button("Configure") { editing = game }.disabled(player?.game.id == game.id)
            #if os(macOS)
                Button("Show in Finder") {
                    if let url = try? game.resolve() { NSWorkspace.shared.activateFileViewerSelecting([url]) }
                }
            #endif
            Button("Remove from Library", role: .destructive) { removing = game }.disabled(player?.game.id == game.id)
        }
    }
    func choose(_ font: Bool, _ copy: Bool) {
        guard font || progress == nil else { return }
        pickFont = font
        copyGame = copy
        picking = true
    }
    func addFont(_ url: URL) {
        Task {
            do {
                let existing = library.data.fonts
                let faces = try await Task.detached { try Library.importFont(url, existing: existing) }.value
                let ids = Set(library.data.fonts.map(\.id))
                library.data.fonts += faces.filter { !ids.contains($0.id) && FileManager.default.fileExists(atPath: $0.url.path) }
            } catch { library.error = .init("Couldn’t Import Font", error.localizedDescription) }
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
                library.data.games.append(game)
            } catch { if !job.isCancelled { library.error = .init("Couldn’t Add Game", error.localizedDescription) } }
        }
    }
    func play(_ game: Game) {
        guard player == nil, editing?.id != game.id else { return }
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
            let id = game.id
            session.didClose = { library.playbackRevision[id, default: 0] += 1 }
            session.closeError = { library.error = .init("Couldn’t Close Game", $0) }
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
            library.error = .init("Couldn’t Open Game", error.localizedDescription)
        }
    }
}

private struct OptionsForm: View {
    @Binding var options: Options
    var fonts: [FontFace]
    let global: Bool
    var game: Game?
    var defaultFont: FontFace?
    var reset: (() -> Void)?
    @AppStorage("showPerformance") private var showPerformance = false
    @State private var showingLicenses = false
    private static let profiles = Bridge.profiles()
    var body: some View {
        Form {
            if let game {
                let info = game.info
                Section {
                    LabeledContent("Resolved Profile", value: info.profile)
                    LabeledContent("Storage") { Text(game.bookmark == nil ? "Library Copy" : "Referenced Folder") }
                    if info.size.count == 2 { LabeledContent("Original Size", value: "\(info.size[0]) × \(info.size[1])") }
                    LabeledContent("Global Default", value: defaultFont?.name ?? String(localized: "No Fonts"))
                }
            }
            Picker("Text Encoding", selection: $options.encoding) {
                Text("Automatic").tag("auto")
                ForEach(GameInfo.encodingLabels, id: \.self) { label in
                    Text(verbatim: label).tag(label.lowercased())
                }
            }
            Picker("Font", selection: $options.font) {
                Text(global ? "Automatic" : "Global Default").tag("")
                ForEach(fonts) { Text($0.name).tag($0.id) }
            }
            Picker("Renderer", selection: $options.metal) {
                Text("Metal").tag(true)
                Text("CPU").tag(false)
            }
            Picker("Scaling", selection: $options.aspect) {
                Text("Game Preference").tag(nil as Int?)
                Text("Original Size").tag(Optional(0))
                Text("Fit").tag(Optional(1))
                Text("Fill").tag(Optional(2))
                Text("Stretch").tag(Optional(3))
            }
            Picker("Scale Filter", selection: $options.scale_filter) {
                Text("Automatic").tag(nil as String?)
                Text("Nearest Neighbor").tag(Optional("nearest"))
                Text("Bilinear").tag(Optional("bilinear"))
                Text("Keys Bicubic").tag(Optional("keys"))
                Text(verbatim: "Mitchell").tag(Optional("mitchell"))
                Text(verbatim: "Lanczos3").tag(Optional("lanczos3"))
                Text("Lanczos3 with Anti-Ringing").tag(Optional("lanczos3-anti-ringing"))
                Text(verbatim: "Lanczos4").tag(Optional("lanczos4"))
            }
            Toggle("Native Audio", isOn: $options.native_audio)
            #if os(macOS)
                Toggle("Use Game Savedata Folder", isOn: $options.game_savedata)
            #else
                Toggle("Multi-Touch Gestures", isOn: $options.gestures)
            #endif
            Section {
                if global { Toggle("Show Performance", isOn: $showPerformance) }
                Picker("Engine Profile", selection: $options.profile) {
                    Text("Automatic").tag("")
                    ForEach(Self.profiles, id: \.self) { Text($0).tag($0) }
                }
                Toggle("Skip Auto-Resume", isOn: $options.skip_auto_resume)
                Toggle("Continue Without Snapshot", isOn: $options.continue_without_snapshot)
                Toggle("Block Presumed Commands", isOn: $options.block_presumed)
                Toggle("Block Unreviewed Commands", isOn: $options.block_unreviewed)
            } header: { Text("Diagnostics") } footer: {
                Text("Use these options to diagnose startup or script problems. They change how the game resumes and which commands it runs.")
            }
            if game != nil {
                Section {
                    Button("Restore Defaults") { reset?() }
                }
            }
            if global {
                Section {
                    Link(destination: URL(string: "https://github.com/Shizuku-in/cmvs-rs")!) {
                        Label("GitHub", systemImage: "chevron.left.forwardslash.chevron.right")
                    }
                    Button("Open-Source Licenses", systemImage: "doc.text") { showingLicenses = true }
                }
            }
        }
        .formStyle(.grouped)
        .sheet(isPresented: $showingLicenses) { LicensesView() }
    }
}

private struct ControlsGuideView: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                #if os(macOS)
                    Section("Pointer") {
                        Label(
                            "Left-click to advance, right-click for the game menu, and scroll to browse history.",
                            systemImage: "cursorarrow.click")
                    }
                    Section("Keyboard & Menu") {
                        Label(
                            "Return confirms and Space hides text. Hold Shift or Ctrl for the game's corresponding fast-forward mode.",
                            systemImage: "keyboard")
                        Label(
                            "Choose Game > Screenshot to capture a frame. Press ⌃⌘F for full screen.",
                            systemImage: "menubar.rectangle")
                    }
                #else
                    Section("Touch") {
                        Label("Tap to advance. Tap the ellipsis for game controls.", systemImage: "hand.tap")
                        Label(
                            "With multi-touch gestures on, tap with two fingers for the game menu, swipe with two to browse history, or tap with three for game controls.",
                            systemImage: "hand.draw")
                    }
                    Section("Keyboard") {
                        Label(
                            "Return confirms and Space hides text. Hold Shift or Ctrl for the game's corresponding fast-forward mode.",
                            systemImage: "keyboard")
                    }
                #endif
            }
            .navigationTitle("Controls Guide")
            #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
        }
        #if os(macOS)
            .frame(minWidth: 460, minHeight: 280)
        #endif
    }
}

private struct ImportProgress: View {
    let progress: Progress
    @State private var bytes: Int64 = 0
    @State private var total: Int64 = -1

    var body: some View {
        VStack(alignment: .leading) {
            if total > 0 {
                ProgressView(value: Double(bytes), total: Double(total)) {
                    Text("Copying \(ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)) of \(ByteCountFormatter.string(fromByteCount: total, countStyle: .file))")
                }
            } else {
                HStack {
                    ProgressView().controlSize(.small)
                    Text("Scanning game folder…")
                }.accessibilityElement(children: .combine)
            }
        }
        .onReceive(progress.publisher(for: \.totalUnitCount).receive(on: DispatchQueue.main)) { total = $0 }
        .onReceive(progress.publisher(for: \.completedUnitCount).receive(on: DispatchQueue.main)) { bytes = $0 }
    }
}

private struct GameConfiguration: View {
    let game: Game
    let library: Library
    @Environment(\.dismiss) private var dismiss
    @State private var options: Options
    @State private var saving = false
    @State private var error: String?

    init(game: Game, library: Library) {
        self.game = game
        self.library = library
        _options = State(initialValue: game.options)
    }

    var body: some View {
        NavigationStack {
            OptionsForm(options: $options, fonts: library.fonts, global: false, game: game,
                        defaultFont: library.defaultFont) {
                options = library.data.defaults
                options.font = ""
            }
            .navigationTitle(game.name)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) { Button("Done") { save() } }
            }
        }
        .disabled(saving).interactiveDismissDisabled(saving || options != game.options)
        #if os(macOS)
            .frame(minWidth: 480, minHeight: 500)
        #endif
        .alert("Couldn’t Save Configuration", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
            Button("OK") { error = nil }
        } message: { Text(error ?? "") }
    }

    private func save() {
        saving = true
        let options = options
        Task {
            defer { saving = false }
            do {
                var info = game.info
                if options.profile != game.options.profile || options.encoding != game.options.encoding {
                    let game = game
                    info = try await Task.detached {
                        let url = try game.resolve()
                        return try scoped(url) { try Library.inspect(url, options: options) }
                    }.value
                }
                if let i = library.data.games.firstIndex(where: { $0.id == game.id }) {
                    library.data.games[i].info = info
                    library.data.games[i].options = options
                }
                dismiss()
            } catch { self.error = error.localizedDescription }
        }
    }
}
