import Accessibility
import CoreTransferable
import SwiftUI
import UniformTypeIdentifiers

struct PlayerScreen: View {
    @Bindable var player: Playback
    @Environment(\.accessibilityVoiceOverEnabled) private var voiceOver
    @Environment(\.scenePhase) private var phase
    @AppStorage("showPerformance") private var showPerformance = false
    @ScaledMetric(relativeTo: .body) private var columnWidth: CGFloat = 232
    #if os(iOS)
        @Environment(\.horizontalSizeClass) private var widthClass
        @Environment(\.verticalSizeClass) private var heightClass
    #endif
    @State private var copied = false

    var body: some View {
        ZStack(alignment: .topTrailing) {
            Color.black.ignoresSafeArea()
            PlayerCanvas(player: player)
                .accessibilityRepresentation { AccessibleCanvas(player: player) }
                .accessibilityHidden(!player.ready || player.error != nil || player.controls || player.dialog != nil)
                #if os(iOS)
                    .ignoresSafeArea(.container)
                #endif
            if !player.ready && player.error == nil {
                VStack {
                    ProgressView().tint(.white).accessibilityLabel("Opening game…")
                    Button("Cancel") { player.finish() }.buttonStyle(.bordered).keyboardShortcut(.cancelAction)
                }.frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            if player.ready && player.error == nil {
                GeometryReader { geometry in
                    controlsButton(in: geometry.size)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)
                }
            }
        }
        .overlay(alignment: .topLeading) {
            VStack(alignment: .leading, spacing: 6) {
                if player.debugRecording != nil {
                    Image(systemName: "record.circle.fill").foregroundStyle(.red)
                        .accessibilityLabel("Recording Diagnostics")
                }
                if showPerformance, player.error == nil, let fps = player.framesPerSecond {
                    let rate = fps.formatted(.number.precision(.fractionLength(1)))
                    let interval = fps > 0 ? (1000 / fps).formatted(.number.precision(.fractionLength(1))) : "—"
                    Text(verbatim: "\(rate) FPS · \(interval) ms")
                        .font(.caption.monospacedDigit()).foregroundStyle(.white)
                        .padding(6).background(.black.opacity(0.75), in: RoundedRectangle(cornerRadius: 6))
                }
            }
            .padding().allowsHitTesting(false)
        }
        .overlay(alignment: .bottom) {
            if let name = player.screenshotName {
                Text("Screenshot saved: \(name)").font(.caption)
                    .lineLimit(1).truncationMode(.middle).padding(.horizontal, 14).padding(.vertical, 8)
                    .background(.regularMaterial, in: Capsule()).padding().allowsHitTesting(false)
            }
        }
        .task(id: player.screenshotName) {
            guard let name = player.screenshotName else { return }
            AccessibilityNotification.Announcement(String(localized: "Screenshot saved: \(name)")).post()
            do { try await Task.sleep(for: .seconds(3)); player.screenshotName = nil } catch {}
        }
        .focusedSceneValue(\.gamePlayer, player)
        .navigationTitle(player.title)
        .onChange(of: voiceOver, initial: true) { _, enabled in player.accessibility.enable(enabled) }
        .task(id: player.accessibility.page?.content.text.map(\.value)) {
            guard voiceOver, let text = player.accessibility.page?.content.text, !text.isEmpty else { return }
            do { try await Task.sleep(for: .milliseconds(450)) } catch { return }
            if phase == .active && !player.controls && player.dialog == nil && player.error == nil {
                AccessibilityNotification.Announcement(text.map(\.value).joined(separator: "\n")).post()
            }
        }
        .onAppear { player.scene(phase == .active) }
        .onChange(of: showPerformance, initial: true) { _, enabled in player.measurePerformance(enabled) }
        .onChange(of: phase) { _, value in player.scene(value == .active) }
        .onDisappear { player.close() }
        .sheet(isPresented: Binding(
            get: { player.dialog != nil && player.error == nil }, set: { if !$0 { player.answer(false) } })
        ) { GameDialog(player: player) }
        .alert("Screenshot Failed", isPresented: Binding(
            get: { player.captureError != nil }, set: { if !$0 { player.captureError = nil } })
        ) { Button("OK") { player.captureError = nil } } message: { Text(player.captureError ?? "") }
        .overlay {
            if let error = player.error {
                let panel = VStack(alignment: .leading, spacing: 16) {
                    Text("Playback Stopped").font(.headline)
                    Text(error).font(.body.monospaced()).textSelection(.enabled)
                    if let report = player.debugReport { DebugReportActions(report: report) }
                    ViewThatFits(in: .horizontal) {
                        HStack { errorActions(error) }.fixedSize(horizontal: true, vertical: false)
                        VStack(alignment: .leading) { errorActions(error) }
                    }
                }.padding().frame(maxWidth: 560, alignment: .leading)
                ViewThatFits(in: .vertical) {
                    panel.fixedSize(horizontal: false, vertical: true)
                    ScrollView { panel }
                }
                .frame(maxWidth: 560)
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12)).padding()
            }
        }
        .environment(\.colorScheme, .dark)
    }

    @ViewBuilder private func errorActions(_ error: String) -> some View {
        Button(copied ? "Copied" : "Copy Diagnostics") {
            let renderer = player.game.options.metal ? String(localized: "Metal") : String(localized: "CPU")
            let report = [
                "\(String(localized: "Game")): \(player.game.name)",
                "\(String(localized: "Engine Profile")): \(player.game.info.profile)",
                "\(String(localized: "Renderer")): \(renderer)",
                "\(String(localized: "Text Encoding")): \(player.encoding)", error,
            ].joined(separator: "\n")
            #if os(macOS)
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(report, forType: .string)
            #else
                UIPasteboard.general.string = report
            #endif
            copied = true
        }.buttonStyle(.bordered)
        Button("Return to Library") { player.finish() }.buttonStyle(.borderedProminent)
    }

    private func controlsButton(in size: CGSize) -> some View {
        HStack(spacing: 6) {
            #if os(iOS)
                if player.interrupted { Text("Audio paused").font(.caption) }
            #endif
            if player.lockedFastForward.contains(.shift) { Text("Shift Fast-Forward").font(.caption) }
            if player.lockedFastForward.contains(.control) { Text("Ctrl Fast-Forward").font(.caption) }
            Button { player.controls = true } label: {
                Image(systemName: "ellipsis.circle.fill").font(.title2)
                    .symbolRenderingMode(.palette).foregroundStyle(.white, .black.opacity(0.7))
                    .controlHitArea()
            }
            .buttonStyle(.borderless)
            #if os(iOS)
                .hoverEffect(.highlight)
            #else
                .help("Show game controls")
            #endif
            .accessibilityLabel("Show game controls")
            .popover(isPresented: $player.controls) {
                #if os(iOS)
                    if widthClass == .compact || heightClass == .compact {
                        NavigationStack {
                            ScrollView { controls(in: size) }
                                .navigationTitle("Game Controls")
                                .navigationBarTitleDisplayMode(.inline)
                                .toolbar {
                                    ToolbarItem(placement: .confirmationAction) {
                                        Button("Done") { player.controls = false }.keyboardShortcut(.cancelAction)
                                    }
                                }
                        }
                        .presentationCompactAdaptation(.sheet)
                        .preferredColorScheme(.dark)
                    } else { controlsPopover(in: size) }
                #else
                    controlsPopover(in: size)
                #endif
            }
        }.padding(8)
    }

    private func controlsPopover(in size: CGSize) -> some View {
        let maxHeight = max(size.height - 76, 1)
        return ViewThatFits(in: .vertical) {
            controls(in: size).fixedSize(horizontal: false, vertical: true)
            ScrollView { controls(in: size) }.frame(height: maxHeight)
        }
        .frame(width: min(max(size.width - 48, 1), usesColumns(size) ? 2 * columnWidth + 48 : columnWidth + 32))
        .frame(maxHeight: maxHeight)
        .fixedSize(horizontal: false, vertical: true)
        .onKeyPress(.escape) {
            player.controls = false
            return .handled
        }
        .preferredColorScheme(.dark)
    }

    private func usesColumns(_ size: CGSize) -> Bool {
        size.width >= 2 * columnWidth + 72 && size.width > size.height
    }

    private func controls(in size: CGSize) -> some View {
        let twoColumns = usesColumns(size)
        let layout = twoColumns ? AnyLayout(HStackLayout(alignment: .top, spacing: 16)) : AnyLayout(VStackLayout(alignment: .leading, spacing: 8))
        return layout {
            VStack(alignment: .leading, spacing: 4) {
                action("Confirm", icon: "return") { player.pulse(22) }
                action("Hide Text", icon: "text.bubble") { player.pulse(26) }
                action("Previous History Entry", icon: "chevron.up") { player.pulse(4) }
                action("Next History Entry", icon: "chevron.down") { player.pulse(5) }
            }
            if !twoColumns { Divider() }
            VStack(alignment: .leading, spacing: 6) {
                Toggle("Shift Fast-Forward", isOn: player.fastForwardBinding(.shift)).disabled(!player.canFastForward)
                Toggle("Ctrl Fast-Forward", isOn: player.fastForwardBinding(.control)).disabled(!player.canFastForward)
                Toggle("Show Performance", isOn: $showPerformance)
                Divider()
                action("Screenshot", icon: "camera") { player.capture() }.disabled(!player.canCapture)
                if player.debugRecording != nil {
                    action("Stop and Export Debug Recording", icon: "stop.circle", dismissControls: false) {
                        player.stopDebugRecording()
                    }
                } else {
                    if let report = player.debugReport { DebugReportActions(report: report) }
                    action("Start Debug Recording", icon: "record.circle") { player.startDebugRecording() }
                }
                #if os(iOS)
                    if player.interrupted {
                        action("Resume Audio", icon: "speaker.wave.2") { player.resumeAudio() }
                    }
                    if let message = player.audioError { Text(message) }
                #endif
                action("Return to Library", icon: "books.vertical") { player.finish() }
            }
        }.padding()
    }

    private func action(
        _ title: LocalizedStringKey, icon: String, dismissControls: Bool = true, perform: @escaping () -> Void
    ) -> some View {
        Button {
            if dismissControls { player.controls = false }
            perform()
        } label: {
            Label(title, systemImage: icon).frame(maxWidth: .infinity, alignment: .leading).controlHitArea()
        }
        #if os(iOS)
            .buttonStyle(.borderless).hoverEffect(.highlight)
        #else
            .buttonStyle(.bordered)
        #endif
    }
}

extension DebugReport: Transferable {
    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(exportedContentType: .json) { report in
            try SentTransferredFile(report.exportFile())
        }
    }
}

private struct DebugReportActions: View {
    let report: DebugReport

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("\(report.summary.eventCount) events · \(report.summary.completedFrames) frames")
                .font(.caption).foregroundStyle(.secondary)
            ShareLink(item: report, preview: SharePreview("Debug Recording")) {
                Label("Export Debug Recording…", systemImage: "square.and.arrow.up").controlHitArea()
            }
        }
    }
}

private struct GameDialog: View {
    @Bindable var player: Playback
    @FocusState private var editing: Bool

    private var kind: String? { player.dialog?["kind"] as? String }

    var body: some View {
        NavigationStack {
            Group {
                switch kind {
                case "comment":
                    ScrollView {
                        VStack(alignment: .leading, spacing: 8) {
                            TextField("Save Comment", text: $player.draft)
                                .textFieldStyle(.roundedBorder).focused($editing)
                                .onAppear { editing = true }.onSubmit { player.answer(true) }
                            let status = player.commentStatus
                            let bytes = status.bytes.map { String($0) } ?? "—"
                            Text("\(status.characters)/\(status.limit) characters · \(bytes)/\(status.byteLimit) bytes")
                                .font(.caption).foregroundStyle(.secondary)
                            if let problem = status.problem { Text(problem).font(.callout).foregroundStyle(.red) }
                        }.frame(maxWidth: .infinity, alignment: .leading).padding()
                    }.navigationTitle("Save Comment")
                case "font":
                    List {
                        if player.dialogFont == nil, let name = player.dialog?["face"] as? String {
                            LabeledContent("Selected", value: name)
                        }
                        ForEach(player.fonts) { face in
                            Button { player.answer(true, face: face) } label: {
                                HStack {
                                    VStack(alignment: .leading) {
                                        FontDetails(face: face)
                                        Text(face.file.hasPrefix("/") ? "System Font" : "Imported Font").font(.caption)
                                    }
                                    Spacer()
                                    if player.dialogFont?.id == face.id {
                                        Image(systemName: "checkmark").accessibilityLabel("Selected")
                                    }
                                }.contentShape(Rectangle())
                            }.buttonStyle(.plain)
                        }
                    }.navigationTitle("Font")
                default:
                    ScrollView {
                        Text(player.dialog?["text"] as? String ?? "")
                            .frame(maxWidth: .infinity, alignment: .leading).padding()
                    }.navigationTitle(player.dialog?["title"] as? String ?? "")
                }
            }
            #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    if kind == "comment" || kind == "font" || player.dialog?["confirm"] as? Bool == true {
                        Button("Cancel") { player.answer(false) }.keyboardShortcut(.cancelAction)
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    if kind == "comment" {
                        Button("Save") { player.answer(true) }
                            .keyboardShortcut(.defaultAction).disabled(player.commentStatus.problem != nil)
                    } else if kind != "font" {
                        Button("OK") { player.answer(true) }.keyboardShortcut(.defaultAction)
                    }
                }
            }
        }
        .interactiveDismissDisabled()
        #if os(macOS)
            .frame(minWidth: 420, minHeight: kind == "font" ? 360 : 180)
        #endif
    }
}
