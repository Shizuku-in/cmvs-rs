import CmvsAppleC
import Foundation
import ImageIO
import QuartzCore
import UniformTypeIdentifiers

final class Bridge {
    let handle = cmvs_apple_create()!
    deinit { cmvs_apple_destroy(handle) }
    var text: String { String(cString: cmvs_apple_text(handle)) }
    func check(_ status: Int32) throws {
        if status < 0 { throw PlayerError(engineMessage: text) }
    }
    func object() throws -> [String: Any] {
        guard let value = try JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any] else {
            throw PlayerError("Invalid engine response.")
        }
        return value
    }
    static func profiles() -> [String] {
        let api = Bridge()
        guard cmvs_apple_list_profiles(api.handle) == 0,
            let items = try? JSONSerialization.jsonObject(with: Data(api.text.utf8)) as? [[String: Any]]
        else { return [] }
        return items.compactMap { $0["id"] as? String }
    }
}

func rgbaImage(_ data: Data, width: Int, height: Int) -> CGImage? {
    guard width > 0, height > 0, data.count == width * height * 4,
        let provider = CGDataProvider(data: data as CFData)
    else { return nil }
    return CGImage(
        width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32,
        bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue),
        provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
}

struct StopDiagnostic: Decodable, LocalizedError {
    let script, reason: String
    let pc, steps: UInt64
    var errorDescription: String? {
        let location = String(localized: "\(script) · PC 0x\(String(pc, radix: 16)) · \(steps) steps")
        return "\(location)\n\(reason)"
    }
}

final class Engine {
    // Closing a still-loading game must finish before the next session opens its saves.
    private static let channel = DispatchQueue(label: "org.cmvs.player.engine", qos: .userInteractive)
    var queue: DispatchQueue { Self.channel }
    // Only accessed on queue, including destruction.
    private(set) var api: Bridge?
    private var scope: URL?
    private var retainedLayer: CAMetalLayer?
    private var size = [0, 0]
    func open(
        _ game: Game, font: FontFace, fonts: [FontFace], layer: CAMetalLayer, width: UInt32, height: UInt32
    ) throws -> [String: Any] {
        let root = try game.resolve()
        if root.startAccessingSecurityScopedResource() { scope = root }
        do {
            let bridge = Bridge()
            var config = try JSONSerialization.jsonObject(with: JSONEncoder().encode(game.options)) as! [String: Any]
            config["root"] = root.path
            config["user"] = game.saves.path
            config["font"] = font.url.path
            config["face"] = font.index
            config["fonts"] = fonts.map { face -> [String: Any] in
                var entry: [String: Any] = ["id": face.id, "name": face.name, "path": face.url.path, "index": face.index]
                if let systemName = face.systemName { entry["system"] = systemName }
                return entry
            }
            let json = try JSONSerialization.data(withJSONObject: config)
            try bridge.check(cmvs_apple_open(bridge.handle, String(decoding: json, as: UTF8.self)))
            let info = try bridge.object()
            size = info["size"] as! [Int]
            if game.options.metal {
                retainedLayer = layer
                try bridge.check(
                    cmvs_apple_attach_metal_layer(
                        bridge.handle, Unmanaged.passUnretained(layer).toOpaque(), width, height))
            }
            api = bridge
            return info
        } catch {
            retainedLayer = nil
            scope?.stopAccessingSecurityScopedResource()
            scope = nil
            throw error
        }
    }
    func image(presentation: Bool = false) throws -> CGImage {
        guard let api else { throw PlayerError("Game closed.") }
        var pixels: UnsafePointer<UInt8>?
        var dimensions = size
        if presentation {
            var output = [UInt32](repeating: 0, count: 2)
            try api.check(cmvs_apple_read_presentation(api.handle, &pixels, &output))
            dimensions = output.map(Int.init)
        } else {
            try api.check(cmvs_apple_read_frame(api.handle, &pixels))
        }
        let data = Data(bytes: pixels!, count: dimensions[0] * dimensions[1] * 4)
        guard let image = rgbaImage(data, width: dimensions[0], height: dimensions[1]) else {
            throw PlayerError("Invalid frame.")
        }
        return image
    }
    func screenshot(to url: URL) throws {
        let image = try image()
        guard let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil) else {
            throw PlayerError("Cannot create screenshot.")
        }
        CGImageDestinationAddImage(dest, image, nil)
        guard CGImageDestinationFinalize(dest) else { throw PlayerError("Cannot save screenshot.") }
    }
    func close(save: Bool) throws {
        defer {
            api = nil
            retainedLayer = nil
            scope?.stopAccessingSecurityScopedResource()
            scope = nil
        }
        if let api {
            defer { _ = cmvs_apple_detach(api.handle) }
            if save { try api.check(cmvs_apple_save_preferences(api.handle)) }
        }
    }
}
