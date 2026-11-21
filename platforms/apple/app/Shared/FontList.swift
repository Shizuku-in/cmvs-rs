import CoreText
import SwiftUI

private actor FontPreviewCache {
    static let shared = FontPreviewCache()
    private let fonts = NSCache<NSString, CTFont>()

    func preview(for face: FontFace, size: CGFloat) -> CTFont? {
        let key = "\(face.id)@\(size)" as NSString
        if let cached = fonts.object(forKey: key) { return cached }
        guard !Task.isCancelled, let font = face.preview(size: size) else { return nil }
        fonts.setObject(font, forKey: key)
        return font
    }
}

struct FontDetails: View {
    let face: FontFace
    @ScaledMetric(relativeTo: .body) private var size: CGFloat = 18
    @State private var preview: CTFont?

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(face.name)
            if let preview { Text(verbatim: "日本語 · 简体中文").font(Font(preview)) }
            Text(verbatim: "\(ByteCountFormatter.string(fromByteCount: Int64(face.bytes), countStyle: .file)) · #\(face.index)")
                .font(.caption).foregroundStyle(.secondary)
        }
        .task(id: "\(face.id)@\(size)") {
            let font = await FontPreviewCache.shared.preview(for: face, size: size)
            if !Task.isCancelled { preview = font }
        }
    }
}

struct FontList: View {
    let library: Library
    let playing: Bool
    let pick: () -> Void
    @State private var search = ""
    @State private var removing: FontFace?

    var body: some View {
        Group {
            if library.data.fonts.isEmpty && library.systemFonts.isEmpty {
                ContentUnavailableView {
                    Label("No Fonts", systemImage: "f.cursive")
                } description: {
                    Text("Import a font before starting a game.")
                } actions: { Button("Import Font…", action: pick) }
            } else {
                let imported = matches(library.data.fonts)
                let system = matches(library.systemFonts)
                List {
                    section("Imported Fonts", faces: imported, imported: true)
                    section("System Fonts", faces: system, imported: false)
                }
                .overlay {
                    if imported.isEmpty && system.isEmpty { ContentUnavailableView.search(text: search) }
                }
                .searchable(text: $search)
            }
        }
        .toolbar {
            Button(action: pick) {
                Label("Import Font…", systemImage: "plus")
            }
            .help("Import Font…")
        }
        .confirmationDialog("Delete Font File?", isPresented: Binding(
            get: { removing != nil }, set: { if !$0 { removing = nil } }
        ), titleVisibility: .visible, presenting: removing) { face in
            Button("Delete", role: .destructive) {
                do { try library.removeFont(face) } catch { library.error = .init("Couldn’t Delete Font", error.localizedDescription) }
            }
        } message: { face in Text("All faces in the file containing \(face.name) will be removed.") }
    }

    private func matches(_ faces: [FontFace]) -> [FontFace] {
        search.isEmpty ? faces : faces.filter { $0.name.localizedCaseInsensitiveContains(search) }
    }

    @ViewBuilder private func section(_ title: LocalizedStringKey, faces: [FontFace], imported: Bool) -> some View {
        if !faces.isEmpty {
            Section(title) {
                ForEach(faces) { face in
                    let selected = library.defaultFont?.id == face.id
                    Button { library.data.defaults.font = face.id } label: {
                        HStack {
                            FontDetails(face: face)
                            Spacer()
                            if selected { Image(systemName: "checkmark").foregroundStyle(.tint) }
                        }.frame(maxWidth: .infinity, alignment: .leading).controlHitArea()
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Set \(face.name) as Default")
                    .accessibilityAddTraits(selected ? .isSelected : [])
                    .contextMenu { if imported { deleteButton(face) } }
                    .swipeActions { if imported { deleteButton(face) } }
                }
            }
        }
    }

    private func deleteButton(_ face: FontFace) -> some View {
        Button("Delete", systemImage: "trash", role: .destructive) { removing = face }
            .accessibilityLabel("Delete \(face.name)").disabled(playing)
    }
}
