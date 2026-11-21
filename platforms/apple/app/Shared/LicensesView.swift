import SwiftUI

struct DependencyCatalog: Equatable {
    struct Package: Identifiable, Equatable {
        let name: String
        let version: String
        let license: String
        let shared: [String]
        let notice: String
        var id: String { "\(name)@\(version)" }
    }

    struct SharedLicense: Identifiable, Equatable {
        let name: String
        let text: String
        var id: String { name }
    }

    let packages: [Package]
    let shared: [SharedLicense]

    func detailText(for package: Package) -> String {
        var sections = ["\(package.name) \(package.version) — \(package.license)"]
        if !package.notice.isEmpty { sections.append(package.notice) }
        let sharedByName = Dictionary(uniqueKeysWithValues: shared.map { ($0.name, $0.text) })
        for key in package.shared {
            if let text = sharedByName[key], !text.isEmpty {
                sections.append("--- \(key) ---\n\n\(text)")
            } else {
                sections.append("License: \(key)")
            }
        }
        return sections.joined(separator: "\n\n")
    }

    static func parse(_ text: String) -> DependencyCatalog {
        enum Section { case none, packages, shared }
        var section = Section.none
        var packages = [Package]()
        var shared = [SharedLicense]()
        var pendingPackage: (name: String, version: String, license: String, shared: [String], lines: [String])?
        var pendingShared: (name: String, lines: [String])?

        func flushPackage() {
            guard let item = pendingPackage else { return }
            pendingPackage = nil
            let notice = item.lines.joined(separator: "\n").trimmingCharacters(in: .newlines)
            packages.append(Package(name: item.name, version: item.version, license: item.license,
                                    shared: item.shared, notice: notice))
        }

        func flushShared() {
            guard let item = pendingShared else { return }
            pendingShared = nil
            let body = item.lines.joined(separator: "\n").trimmingCharacters(in: .newlines)
            shared.append(SharedLicense(name: item.name, text: body))
        }

        for rawLine in text.replacingOccurrences(of: "\r\n", with: "\n").split(separator: "\n", omittingEmptySubsequences: false) {
            let line = String(rawLine)
            if line == "=== PACKAGES ===" {
                flushPackage()
                flushShared()
                section = .packages
            } else if line == "=== SHARED LICENSES ===" {
                flushPackage()
                flushShared()
                section = .shared
            } else if section == .packages {
                if line.hasPrefix("== "), line.hasSuffix(" =="), line.count > 6 {
                    let inner = String(line.dropFirst(3).dropLast(3))
                    if let dash = inner.range(of: " — ", options: .backwards) {
                        let left = String(inner[..<dash.lowerBound])
                        let license = String(inner[dash.upperBound...])
                        if let space = left.lastIndex(of: " ") {
                            flushPackage()
                            pendingPackage = (String(left[..<space]), String(left[left.index(after: space)...]),
                                              license, [], [])
                            continue
                        }
                    }
                }
                if var item = pendingPackage {
                    if item.shared.isEmpty, item.lines.isEmpty, line.hasPrefix("Shared: ") {
                        item.shared = line.dropFirst(8).split(separator: ",")
                            .map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
                    } else {
                        item.lines.append(line)
                    }
                    pendingPackage = item
                }
            } else if section == .shared {
                if line.hasPrefix("== ["), line.hasSuffix("] =="), line.count > 8 {
                    flushShared()
                    pendingShared = (String(line.dropFirst(4).dropLast(4)), [])
                } else if var item = pendingShared {
                    item.lines.append(line)
                    pendingShared = item
                }
            }
        }
        flushPackage()
        flushShared()
        return DependencyCatalog(packages: packages, shared: shared)
    }
}

struct LicensesView: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                document("MIT", file: "LICENSE-MIT")
                document("Apache 2.0", file: "LICENSE-APACHE")
                document("Third-Party Notices", file: "THIRD_PARTY_NOTICES.txt")
                NavigationLink("Dependency Licenses") {
                    DependencyLicensesView { dismiss() }.navigationTitle("Dependency Licenses")
                }
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

private struct DependencyLicensesView: View {
    let close: () -> Void
    @State private var raw: String?
    @State private var catalog: DependencyCatalog?
    @State private var search = ""

    private var done: some ToolbarContent {
        ToolbarItem(placement: .confirmationAction) { Button("Done", action: close) }
    }

    private var filteredPackages: [DependencyCatalog.Package] {
        let all = catalog?.packages ?? []
        guard !search.isEmpty else { return all }
        return all.filter {
            $0.name.localizedCaseInsensitiveContains(search) || $0.license.localizedCaseInsensitiveContains(search)
        }
    }

    private var filteredShared: [DependencyCatalog.SharedLicense] {
        let all = catalog?.shared ?? []
        guard !search.isEmpty else { return all }
        return all.filter { $0.name.localizedCaseInsensitiveContains(search) }
    }

    var body: some View {
        Group {
            if let raw {
                if raw.isEmpty {
                    ContentUnavailableView("License Unavailable", systemImage: "doc")
                } else if let catalog, !catalog.packages.isEmpty {
                    List {
                        if !filteredPackages.isEmpty {
                            Section("Dependencies") {
                                ForEach(filteredPackages) { pkg in
                                    NavigationLink {
                                        LicenseDocument(text: catalog.detailText(for: pkg))
                                            .navigationTitle(pkg.name)
                                            .toolbar { done }
                                    } label: {
                                        VStack(alignment: .leading, spacing: 2) {
                                            Text(verbatim: pkg.name)
                                            Text(verbatim: "\(pkg.version) · \(pkg.license)")
                                                .font(.caption).foregroundStyle(.secondary)
                                        }
                                    }
                                }
                            }
                        }
                        if !filteredShared.isEmpty {
                            Section("Shared Licenses") {
                                ForEach(filteredShared) { item in
                                    NavigationLink {
                                        LicenseDocument(text: item.text)
                                            .navigationTitle(item.name)
                                            .toolbar { done }
                                    } label: {
                                        Text(verbatim: item.name)
                                    }
                                }
                            }
                        }
                    }
                    .overlay {
                        if filteredPackages.isEmpty && filteredShared.isEmpty {
                            ContentUnavailableView.search(text: search)
                        }
                    }
                    .searchable(text: $search)
                } else {
                    LicenseDocument(text: raw)
                }
            } else {
                ProgressView()
            }
        }
        .toolbar { done }
        .task {
            let loaded: (String, DependencyCatalog?) = await Task.detached {
                let text = loadBundleText("DEPENDENCY-LICENSES.txt")
                return (text, text.isEmpty ? nil : DependencyCatalog.parse(text))
            }.value
            raw = loaded.0
            catalog = loaded.1
        }
    }
}

private func loadBundleText(_ file: String) -> String {
    guard let url = Bundle.main.url(forResource: file, withExtension: nil),
        let data = try? Data(contentsOf: url) else { return "" }
    return String(decoding: data, as: UTF8.self)
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
            text = await Task.detached { loadBundleText(file) }.value
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
