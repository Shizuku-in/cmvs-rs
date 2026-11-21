import SwiftUI

struct PlayerScreen: View {
    @Bindable var player: Playback
    @Environment(\.scenePhase) private var phase
    var body: some View {
        ZStack(alignment: .topTrailing) {
            Color.black
            PlayerCanvas(player: player)
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
                    Button("Menu") { player.pulse(3) }
                    Button(player.fast ? "Release fast-forward" : "Fast-forward") { player.toggleFast() }
                    Text("Hold to skip")
                        .padding(4)
                        .onLongPressGesture(
                            minimumDuration: 0.15, perform: {},
                            onPressingChanged: { held in
                                if !player.fast { player.key(24, held) }
                            })
                    Button("Confirm") { player.pulse(22) }
                    Button("Hide text") { player.pulse(26) }
                    Button("Screenshot") { player.capture() }.disabled(!player.canCapture)
                    #if os(iOS)
                        if player.interrupted { Button("Resume audio") { player.resumeAudio() } }
                        if let message = player.audioError { Text(message) }
                    #endif
                    Button("Return to library") { player.finish() }
                }
            }.padding(8).background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10)).padding()
        }
        .navigationTitle(player.title)
        .onAppear { player.scene(phase == .active) }
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
}
