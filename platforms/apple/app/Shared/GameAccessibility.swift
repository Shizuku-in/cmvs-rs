import CmvsAppleC
import SwiftUI
import Vision

struct GamePage: Decodable {
    struct Choice: Decodable, Identifiable {
        let id: [Int]
        let rect: [Int]
        let label: String
        let slider: UInt8?
    }
    struct TextBlock: Decodable, Identifiable {
        let id: Int
        let value: String
        let rect: [Int]
    }
    struct Content: Decodable {
        let modal: Bool
        let text: [TextBlock]
        let choices: [Choice]
    }
    let revision: UInt64
    let content: Content
}

// Some games bake option titles into sprites. Read those locally, off the render queue.
private actor BitmapTitleReader {
    func labels(in image: CGImage, choices: [GamePage.Choice]) -> [[Int]: String] {
        guard !Task.isCancelled else { return [:] }
        let request = VNRecognizeTextRequest()
        request.automaticallyDetectsLanguage = true
        request.recognitionLevel = .accurate
        do { try VNImageRequestHandler(cgImage: image, options: [:]).perform([request]) } catch { return [:] }

        var labels = [[Int]: String]()
        for observation in request.results ?? [] {
            guard let text = observation.topCandidates(1).first?.string.trimmingCharacters(in: .whitespacesAndNewlines),
                !text.isEmpty else { continue }
            let box = observation.boundingBox
            let point = CGPoint(x: box.midX * CGFloat(image.width), y: (1 - box.midY) * CGFloat(image.height))
            for choice in choices {
                let r = choice.rect
                if CGRect(x: r[0], y: r[1], width: r[2], height: r[3]).contains(point) {
                    labels[choice.id] = labels[choice.id].map { $0 + " " + text } ?? text
                }
            }
        }
        return labels
    }
}

@MainActor @Observable final class GameAccessibility {
    private(set) var page: GamePage?
    private(set) var bitmapLabels = [[Int]: String]()
    @ObservationIgnored private var enabled = false
    @ObservationIgnored private var nextRead: CFTimeInterval = 0
    @ObservationIgnored private var nextRecognition: CFTimeInterval = 0
    @ObservationIgnored private var recognition: Task<Void, Never>?
    private let reader = BitmapTitleReader()

    func enable(_ value: Bool) {
        enabled = value
        nextRead = 0
        nextRecognition = 0
        recognition?.cancel()
        recognition = nil
        page = nil
        bitmapLabels = [:]
    }

    func update(at time: CFTimeInterval, engine: Engine, cpuFrame: CGImage?) {
        guard enabled, time >= nextRead else { return }
        nextRead = time + 0.2
        let known = page?.revision ?? 0
        let updated = engine.queue.sync { () -> GamePage? in
            guard let api = engine.api, cmvs_apple_accessibility(api.handle, known) == 1 else { return nil }
            return try? JSONDecoder().decode(GamePage.self, from: Data(api.text.utf8))
        }
        if let updated {
            recognition?.cancel()
            recognition = nil
            bitmapLabels = [:]
            page = updated
            nextRecognition = time + 0.3
        }
        guard recognition == nil, let page, time >= nextRecognition else { return }
        let missing = page.content.choices.filter { $0.label.isEmpty && bitmapLabels[$0.id] == nil }
        guard !missing.isEmpty else { return }
        nextRecognition = time + 2
        // Capture this completed frame, before the next VM tick changes the labels.
        guard let image = cpuFrame ?? engine.queue.sync(execute: { try? engine.image() }) else { return }
        recognition = Task {
            let labels = await reader.labels(in: image, choices: missing)
            guard !Task.isCancelled, self.page?.revision == page.revision else { return }
            if !labels.isEmpty { bitmapLabels.merge(labels) { _, new in new } }
            recognition = nil
        }
    }
}

struct AccessibleCanvas: View {
    let player: Playback

    var body: some View {
        if let page = player.accessibility.page {
            ZStack(alignment: .topLeading) {
                ForEach(page.content.text) { block in
                    let frame = player.canvasRect(block.rect)
                    let text = Text(verbatim: block.value)
                        .frame(width: frame.width, height: frame.height)
                        .position(x: frame.midX, y: frame.midY)
                    if page.content.modal { text } else { text.accessibilityAction { player.pulse(22) } }
                }
                ForEach(Array(page.content.choices.enumerated()), id: \.element.id) { index, choice in
                    let frame = player.canvasRect(choice.rect)
                    let choose: (Int32) -> Void = { value in
                        player.invoke { cmvs_apple_choose($0, page.revision, UInt32(index), value) }
                    }
                    Group {
                        if let slider = choice.slider {
                            Slider(value: Binding(
                                get: { Double(slider) }, set: { choose(Int32($0.rounded())) }
                            ), in: 0...255, step: 1) { Text(verbatim: choice.label) }
                                .accessibilityValue(Text(verbatim: String(slider)))
                        } else {
                            Button { choose(-1) } label: {
                                if !choice.label.isEmpty {
                                    Text(verbatim: choice.label)
                                } else if let title = player.accessibility.bitmapLabels[choice.id] {
                                    Text(verbatim: title)
                                } else { Text("Option \(index + 1)") }
                            }
                        }
                    }
                    .frame(width: frame.width, height: frame.height)
                    .position(x: frame.midX, y: frame.midY)
                }
            }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .accessibilityElement(children: .contain)
        }
    }
}
