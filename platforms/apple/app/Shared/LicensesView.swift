import SwiftUI

struct LicensesView: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                document("MIT", file: "LICENSE-MIT")
                document("Apache 2.0", file: "LICENSE-APACHE")
                document("Third-Party Notices", file: "THIRD_PARTY_NOTICES.txt")
                document("Dependency Licenses", file: "DEPENDENCY-LICENSES.txt")
            }
            .navigationTitle("Open-Source Licenses")
            .toolbar { done }
        }
        #if os(macOS)
            .frame(minWidth: 500, minHeight: 360)
        #endif
    }

    private var done: some ToolbarContent {
        ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
    }

    private func document(_ title: LocalizedStringKey, file: String) -> some View {
        NavigationLink {
            LicenseText(file: file).navigationTitle(title).toolbar { done }
        } label: {
            Text(title)
        }
    }
}

private struct LicenseText: View {
    let file: String
    @State private var text: String?

    var body: some View {
        Group {
            if let text {
                if text.isEmpty {
                    ContentUnavailableView("License Unavailable", systemImage: "doc")
                } else {
                    LicenseDocument(text: text)
                }
            } else {
                ProgressView()
            }
        }
        .task(id: file) {
            text = await Task.detached {
                guard let url = Bundle.main.url(forResource: file, withExtension: nil),
                    let data = try? Data(contentsOf: url) else { return "" }
                return String(decoding: data, as: UTF8.self)
            }.value
        }
    }
}

#if os(macOS)
    private struct LicenseDocument: NSViewRepresentable {
        let text: String
        func makeNSView(context: Context) -> NSScrollView {
            let scroll = NSTextView.scrollableTextView()
            let view = scroll.documentView as! NSTextView
            view.isEditable = false
            view.usesFindBar = true
            view.isIncrementalSearchingEnabled = true
            view.font = .monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
            view.textContainerInset = NSSize(width: 12, height: 8)
            view.string = text
            return scroll
        }
        func updateNSView(_ view: NSScrollView, context: Context) {}
    }
#else
    private struct LicenseDocument: UIViewRepresentable {
        let text: String
        func makeUIView(context: Context) -> UITextView {
            let view = UITextView()
            view.isEditable = false
            view.backgroundColor = .clear
            view.textContainerInset = UIEdgeInsets(top: 10, left: 16, bottom: 10, right: 16)
            let font = UIFont.monospacedSystemFont(ofSize: 17, weight: .regular)
            view.font = UIFontMetrics(forTextStyle: .body).scaledFont(for: font)
            view.adjustsFontForContentSizeCategory = true
            view.text = text
            return view
        }
        func updateUIView(_ view: UITextView, context: Context) {}
    }
#endif
