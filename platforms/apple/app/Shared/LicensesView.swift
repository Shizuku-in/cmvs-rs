import SwiftUI

struct LicensesView: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                document("MIT", file: "LICENSE-MIT")
                document("Apache 2.0", file: "LICENSE-APACHE")
                document("Third-party notices", file: "THIRD_PARTY_NOTICES.txt")
                document("Dependency licenses", file: "DEPENDENCY-LICENSES.txt")
            }
            .navigationTitle("Open-source licenses")
            .toolbar { Button("Done") { dismiss() } }
        }
        #if os(macOS)
            .frame(minWidth: 500, minHeight: 360)
        #endif
    }

    private func document(_ title: LocalizedStringKey, file: String) -> some View {
        NavigationLink {
            LicenseText(file: file).navigationTitle(title)
        } label: {
            Text(title)
        }
    }
}

private struct LicenseText: View {
    let file: String
    @State private var paragraphs: [String]?

    var body: some View {
        Group {
            if let paragraphs {
                if paragraphs.isEmpty {
                    ContentUnavailableView("License unavailable", systemImage: "doc")
                } else {
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 10) {
                            ForEach(paragraphs.indices, id: \.self) { i in
                                Text(verbatim: paragraphs[i]).textSelection(.enabled)
                            }
                        }.padding()
                    }
                }
            } else {
                ProgressView()
            }
        }
        .task(id: file) {
            guard let url = Bundle.main.url(forResource: file, withExtension: nil),
                let data = try? Data(contentsOf: url) else {
                paragraphs = []
                return
            }
            paragraphs = String(decoding: data, as: UTF8.self)
                .replacingOccurrences(of: "\r\n", with: "\n")
                .components(separatedBy: "\n\n")
                .filter { !$0.isEmpty }
        }
    }
}
