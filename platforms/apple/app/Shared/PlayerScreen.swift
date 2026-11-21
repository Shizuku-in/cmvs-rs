import SwiftUI

struct PlayerScreen: View {
    @Bindable var player: Playback
    @Environment(\.scenePhase) private var phase
    @AppStorage("showPerformance") private var showPerformance = false
    var body: some View {
        ZStack(alignment: .topTrailing) {
            Color.black
            PlayerCanvas(player: player)
                .overlay(alignment: .topLeading) {
                    if showPerformance, player.error == nil, let fps = player.framesPerSecond {
                        let rate = fps.formatted(.number.precision(.fractionLength(1)))
                        let interval = fps > 0 ? (1000 / fps).formatted(.number.precision(.fractionLength(1))) : "—"
                        Text(verbatim: "\(rate) FPS · \(interval) ms")
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.white)
                            .padding(6).background(.black.opacity(0.75), in: RoundedRectangle(cornerRadius: 6))
                            .padding().allowsHitTesting(false)
                    }
                }
            if !player.ready && player.error == nil {
                VStack {
                    ProgressView()
                    Button("Cancel") { player.finish() }
                }.frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            VStack(alignment: .trailing) {
                Button {
                    player.controls.toggle()
                } label: {
                    Image(systemName: "ellipsis.circle.fill")
                }
                .accessibilityLabel("Game controls")
                if player.controls {
                    ViewThatFits(in: .vertical) {
                        controls
                        ScrollView { controls }
                    }.fixedSize(horizontal: true, vertical: false)
                }
            }.padding(8).background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10)).padding()
        }
        .navigationTitle(player.title)
        .onAppear { player.scene(phase == .active) }
        .onChange(of: showPerformance, initial: true) { _, enabled in player.measurePerformance(enabled) }
        .onChange(of: phase) { _, value in player.scene(value == .active) }
        .onDisappear { player.close() }
        .sheet(
            isPresented: Binding(
                get: { player.dialog != nil && player.error == nil }, set: { if !$0 { player.answer(false) } })
        ) {
            VStack(alignment: .leading, spacing: 16) {
                switch player.dialog?["kind"] as? String {
                case "comment":
                    TextField("Save comment", text: $player.draft)
                    Button("Save") { player.answer(true) }.disabled(!player.validDraft)
                case "font":
                    List(player.fonts) { face in Button(face.name) { player.answer(true, face: face) } }
                default:
                    Text(player.dialog?["title"] as? String ?? "")
                    Text(player.dialog?["text"] as? String ?? "")
                    Button("OK") { player.answer(true) }
                }
                if player.dialog?["kind"] as? String != "message" || player.dialog?["confirm"] as? Bool == true {
                    Button("Cancel") { player.answer(false) }
                }
            }.padding().interactiveDismissDisabled()
        }
        .alert("Screenshot failed", isPresented: Binding(
            get: { player.captureError != nil }, set: { if !$0 { player.captureError = nil } })
        ) { Button("OK") { player.captureError = nil } } message: { Text(player.captureError ?? "") }
        .overlay {
            if let error = player.error {
                VStack(alignment: .leading, spacing: 16) {
                    Text("Playback stopped").font(.headline)
                    ScrollView { Text(error).font(.body.monospaced()).textSelection(.enabled) }
                    HStack {
                        Button("Copy diagnostics") {
                            #if os(macOS)
                                NSPasteboard.general.clearContents()
                                NSPasteboard.general.setString(error, forType: .string)
                            #else
                                UIPasteboard.general.string = error
                            #endif
                        }
                        Button("Return to library") { player.finish() }
                    }
                }.padding().frame(maxWidth: 560, maxHeight: 320).background(.regularMaterial)
            }
        }
    }

    private var controls: some View {
        VStack(alignment: .trailing) {
            fastForward("Shift fast-forward", key: .shift)
            fastForward("Ctrl fast-forward", key: .control)
            Button("Confirm") { player.pulse(22) }
            Button("Hide text") { player.pulse(26) }
            Button("Screenshot") { player.capture() }.disabled(!player.canCapture)
            Toggle("Show performance", isOn: $showPerformance).toggleStyle(.button)
            #if os(iOS)
                if player.interrupted { Button("Resume audio") { player.resumeAudio() } }
                if let message = player.audioError { Text(message) }
            #endif
            Button("Return to library") { player.finish() }
        }
    }

    private func fastForward(_ title: LocalizedStringKey, key: FastForwardKey) -> some View {
        Toggle(title, isOn: Binding(
            get: { player.lockedFastForward.contains(key) },
            set: { player.setFastForward(key, locked: $0) }
        ))
        .toggleStyle(.button)
        .disabled(!player.canFastForward)
    }
}
