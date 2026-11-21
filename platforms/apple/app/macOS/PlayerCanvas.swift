import CmvsAppleC
import QuartzCore
import SwiftUI

struct PlayerCanvas: NSViewRepresentable {
    var player: Playback
    func makeNSView(context: Context) -> Canvas { Canvas(player) }
    func updateNSView(_ view: Canvas, context: Context) {}
    static func dismantleNSView(_ view: Canvas, coordinator: ()) { view.player.close() }
}

final class Canvas: NSView {
    let player: Playback
    let metal = CAMetalLayer()
    private var gameCursor = NSCursor.arrow
    private let transparent = NSCursor(image: NSImage(size: NSSize(width: 1, height: 1)), hotSpot: .zero)
    private var shown = true
    private var track: NSTrackingArea?
    private var accepted = Set<UInt32>()
    private var started = false
    private var pendingWindowSize: CGSize?
    private var sizedWindow = false
    private var scrollDistance: CGFloat = 0
    override var acceptsFirstResponder: Bool { true }
    override var isFlipped: Bool { true }
    init(_ player: Playback) {
        self.player = player
        super.init(frame: .zero)
        player.clearKeys = { [weak self] in self?.accepted.removeAll() }
        wantsLayer = true
        layer = metal
        metal.backgroundColor = NSColor.black.cgColor
    }
    required init?(coder: NSCoder) { nil }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        NotificationCenter.default.removeObserver(self)
        guard let window else { return }
        window.isRestorable = false
        let size = player.game.info.size
        if !sizedWindow, size.count == 2, size.allSatisfy({ $0 > 0 }), !window.styleMask.contains(.fullScreen) {
            sizedWindow = true
            let visible = (window.screen ?? NSScreen.main)?.visibleFrame ?? window.frame
            let available = window.contentRect(forFrameRect: visible).size
            let w = CGFloat(size[0]), h = CGFloat(size[1])
            let scale = min(1, available.width / w, available.height / h)
            window.setContentSize(CGSize(width: w * scale, height: h * scale))
            let frame = window.frame
            if !visible.contains(frame) {
                window.setFrameOrigin(NSPoint(
                    x: max(visible.minX, min(frame.minX, visible.maxX - frame.width)),
                    y: min(visible.maxY - frame.height, max(frame.minY, visible.minY))))
            }
        }
        window.makeFirstResponder(self)
        window.acceptsMouseMovedEvents = true
        for name in [
            NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification, NSWindow.didMiniaturizeNotification,
            NSWindow.didDeminiaturizeNotification,
        ] {
            NotificationCenter.default.addObserver(self, selector: #selector(focusChanged), name: name, object: window)
        }
        for name in [NSWindow.didEnterFullScreenNotification, NSWindow.didExitFullScreenNotification] {
            NotificationCenter.default.addObserver(self, selector: #selector(fullscreenChanged), name: name, object: window)
        }
        player.windowState = { [weak window] in
            guard let mask = window?.styleMask else { return (false, false) }
            return (!mask.contains(.fullScreen), mask.contains(.resizable))
        }
        player.resizeWindow = { [weak self] pixels in
            self?.pendingWindowSize = pixels
            self?.applyWindowSize()
        }
        player.fullscreen = { [weak window] value in
            if let window, value != window.styleMask.contains(.fullScreen) { window.toggleFullScreen(nil) }
        }
        player.warp = { [weak self] pixels in
            guard let self, let window = self.window, window.isKeyWindow, let main = NSScreen.screens.first else { return }
            let scale = metal.contentsScale
            let point = convert(CGPoint(x: pixels.x / scale, y: pixels.y / scale), to: nil)
            let screen = window.convertPoint(toScreen: point)
            CGWarpMouseCursorPosition(CGPoint(x: screen.x, y: main.frame.maxY - screen.y))
        }
        player.cursorVisible = { [weak self] value in
            guard let self else { return }
            shown = value
            self.window?.invalidateCursorRects(for: self)
        }
        player.cursor = { [weak self] info in
            guard let self else { return }
            if let info, let pixels = info["pixels"] as? [UInt8], let w = info["width"] as? Int,
                let h = info["height"] as? Int,
                let spot = info["hotspot"] as? [Int], spot.count == 2,
                let image = rgbaImage(Data(pixels), width: w, height: h)
            {
                gameCursor = NSCursor(
                    image: NSImage(cgImage: image, size: NSSize(width: w, height: h)),
                    hotSpot: NSPoint(x: spot[0], y: spot[1]))
            } else {
                gameCursor = .arrow
            }
            self.window?.invalidateCursorRects(for: self)
        }
        needsLayout = true
    }
    private func applyWindowSize() {
        guard let window, !window.styleMask.contains(.fullScreen), let pixels = pendingWindowSize else { return }
        pendingWindowSize = nil
        let scale = window.backingScaleFactor
        window.setContentSize(CGSize(width: pixels.width / scale, height: pixels.height / scale))
    }
    @objc private func fullscreenChanged() {
        guard let window else { return }
        player.invoke { cmvs_apple_fullscreen($0, window.styleMask.contains(.fullScreen)) }
        applyWindowSize()
    }
    @objc private func focusChanged() {
        accepted.removeAll()
        player.focus(window?.isKeyWindow == true && window?.isMiniaturized == false)
    }
    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        needsLayout = true
    }
    override func layout() {
        super.layout()
        guard let window, bounds.width > 0, bounds.height > 0 else { return }
        let scale = window.backingScaleFactor
        metal.contentsScale = scale
        metal.drawableSize = CGSize(width: bounds.width * scale, height: bounds.height * scale)
        if started {
            player.resize(bounds.size, scale)
        } else {
            started = true
            player.start(
                metal, size: bounds.size, scale: scale,
                link: displayLink(target: player, selector: #selector(Playback.step(_:))))
        }
    }
    override func resetCursorRects() {
        super.resetCursorRects()
        addCursorRect(bounds, cursor: shown ? gameCursor : transparent)
    }
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let track { removeTrackingArea(track) }
        let area = NSTrackingArea(
            rect: .zero, options: [.activeInKeyWindow, .inVisibleRect, .mouseMoved, .mouseEnteredAndExited],
            owner: self, userInfo: nil)
        track = area
        addTrackingArea(area)
    }
    @discardableResult private func point(_ e: NSEvent) -> Bool {
        let p = convert(e.locationInWindow, from: nil)
        let s = metal.contentsScale
        var inside = false
        player.invoke {
            let result = cmvs_apple_pointer($0, p.x * s, p.y * s)
            inside = result == 1
            return result
        }
        return inside
    }
    private func button(_ e: NSEvent, down: Bool) {
        let codes: [UInt32] = [1, 3, 2, 6, 7]
        guard codes.indices.contains(e.buttonNumber) else { return }
        let code = codes[e.buttonNumber]
        let inside = point(e)
        if down && inside {
            accepted.insert(code)
            player.key(code, true)
        }
        if !down && accepted.remove(code) != nil { player.key(code, false) }
    }
    override func mouseMoved(with e: NSEvent) { point(e) }
    override func mouseExited(with e: NSEvent) { point(e) }
    override func mouseDragged(with e: NSEvent) { point(e) }
    override func otherMouseDragged(with e: NSEvent) { point(e) }
    override func rightMouseDragged(with e: NSEvent) { point(e) }
    override func mouseDown(with e: NSEvent) { button(e, down: true) }
    override func mouseUp(with e: NSEvent) { button(e, down: false) }
    override func rightMouseDown(with e: NSEvent) { button(e, down: true) }
    override func rightMouseUp(with e: NSEvent) { button(e, down: false) }
    override func otherMouseDown(with e: NSEvent) { button(e, down: true) }
    override func otherMouseUp(with e: NSEvent) { button(e, down: false) }
    override func scrollWheel(with e: NSEvent) {
        guard e.momentumPhase.isEmpty, point(e) else { return }
        if e.phase == .began || e.phase == .cancelled { scrollDistance = 0 }
        let delta = e.scrollingDeltaY
        guard delta != 0 else { return }
        if e.hasPreciseScrollingDeltas {
            if scrollDistance * delta < 0 { scrollDistance = 0 }
            scrollDistance += delta
            guard abs(scrollDistance) >= 32 else { return }
            player.pulse(scrollDistance > 0 ? 4 : 5)
            scrollDistance = scrollDistance.truncatingRemainder(dividingBy: 32)
        } else {
            player.pulse(delta > 0 ? 4 : 5)
        }
    }
    override func flagsChanged(with e: NSEvent) {
        player.key(23, e.modifierFlags.contains(.shift))
        player.key(24, e.modifierFlags.contains(.control))
    }
    override func keyDown(with e: NSEvent) {
        if e.modifierFlags.contains(.command) {
            super.keyDown(with: e)
            return
        }
        if let code = Self.keys[e.keyCode], !e.isARepeat { player.key(code, true) }
    }
    override func keyUp(with e: NSEvent) { if let code = Self.keys[e.keyCode] { player.key(code, false) } }
    static let keys: [UInt16: UInt32] = [
        51: 20, 48: 21, 36: 22, 76: 22, 53: 25, 49: 26, 116: 27, 121: 28, 119: 29, 115: 30, 114: 31, 117: 32, 126: 33,
        125: 34, 123: 35, 124: 36,
        82: 37, 83: 38, 84: 39, 85: 40, 86: 41, 87: 42, 88: 43, 89: 44, 91: 45, 92: 46, 67: 47, 69: 48, 78: 49, 65: 50,
        75: 51,
        122: 52, 120: 53, 99: 54, 118: 55, 96: 56, 97: 57, 98: 58, 100: 59, 101: 60, 109: 61, 103: 62, 111: 63, 105: 64,
        107: 65, 113: 66, 106: 67, 64: 68, 79: 69, 80: 70, 90: 71,
        29: 76, 18: 77, 19: 78, 20: 79, 21: 80, 23: 81, 22: 82, 26: 83, 28: 84, 25: 85,
        0: 86, 11: 87, 8: 88, 2: 89, 14: 90, 3: 91, 5: 92, 4: 93, 34: 94, 38: 95, 40: 96, 37: 97, 46: 98, 45: 99,
        31: 100, 35: 101, 12: 102, 15: 103, 1: 104, 17: 105, 32: 106, 9: 107, 13: 108, 7: 109, 16: 110, 6: 111,
        41: 112, 24: 113, 43: 114, 27: 115, 47: 116, 44: 117, 50: 118, 33: 119, 42: 120, 30: 121, 39: 122,
    ]
}
