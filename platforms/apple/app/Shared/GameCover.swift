import ImageIO
import SwiftUI

struct GameCover: View {
    let game: Game
    let revision: Int
    @State private var image: CGImage?

    var body: some View {
        Rectangle().fill(.secondary.opacity(0.1))
            .aspectRatio(16.0 / 9.0, contentMode: .fit)
            .overlay {
                if let image {
                    Image(decorative: image, scale: 1).resizable().scaledToFit()
                } else {
                    Image(systemName: "gamecontroller").foregroundStyle(.secondary)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 6))
            .accessibilityHidden(true)
            .task(id: revision) {
                let url = game.thumbnail
                let decoded = await Task.detached(priority: .utility) { () -> CGImage? in
                    guard FileManager.default.fileExists(atPath: url.path),
                        let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
                    return CGImageSourceCreateThumbnailAtIndex(source, 0, [
                        kCGImageSourceCreateThumbnailFromImageAlways: true,
                        kCGImageSourceShouldCacheImmediately: true,
                        kCGImageSourceThumbnailMaxPixelSize: 480,
                    ] as CFDictionary)
                }.value
                if !Task.isCancelled { image = decoded }
            }
    }
}
