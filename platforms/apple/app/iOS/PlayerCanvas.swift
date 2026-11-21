import CmvsAppleC
import QuartzCore
import SwiftUI

struct PlayerCanvas: UIViewRepresentable {
    var player: Playback
    func makeUIView(context: Context) -> Canvas { Canvas(player) }
    func updateUIView(_ view: Canvas, context: Context) {}
    static func dismantleUIView(_ view: Canvas, coordinator: ()) { view.player.close() }
}

final class Canvas: UIView {
    let player: Playback
    private var started = false
    private var blocked = false
    private var pending: (touch: UITouch, origin: CGPoint)?
    private var pendingPress: Task<Void, Never>?
    private var contacts = Set<UITouch>()
    override class var layerClass: AnyClass { CAMetalLayer.self }
    override var canBecomeFirstResponder: Bool { true }
    init(_ player: Playback) {
        self.player = player
        super.init(frame: .zero)
        player.clearKeys = { [weak self] in
            self?.accepted.removeAll()
            if self?.pending != nil { self?.player.debugRecording?.append(.gesture(decision: .cancelledOnPause)) }
            self?.discardPending()
            if self?.contacts.isEmpty == false { self?.blocked = true }
        }
        isMultipleTouchEnabled = true
        backgroundColor = .black
        let pan = UIPanGestureRecognizer(target: self, action: #selector(history(_:)))
        pan.allowedScrollTypesMask = .all
        if player.game.options.gestures {
            for count in [2, 3] {
                let tap = UITapGestureRecognizer(target: self, action: #selector(tap(_:)))
                tap.numberOfTouchesRequired = count
                addGestureRecognizer(tap)
            }
            pan.minimumNumberOfTouches = 2
            pan.maximumNumberOfTouches = 2
        } else {
            pan.allowedTouchTypes = []
        }
        addGestureRecognizer(pan)
        addGestureRecognizer(UIHoverGestureRecognizer(target: self, action: #selector(hover(_:))))
        let right = UITapGestureRecognizer(target: self, action: #selector(rightClick(_:)))
        right.allowedTouchTypes = [NSNumber(value: UITouch.TouchType.indirectPointer.rawValue)]
        right.buttonMaskRequired = .secondary
        addGestureRecognizer(right)
    }
    required init?(coder: NSCoder) { nil }
    override func layoutSubviews() {
        super.layoutSubviews()
        guard let window, bounds.width > 0, bounds.height > 0 else { return }
        let s = window.screen.scale
        let metal = layer as! CAMetalLayer
        metal.contentsScale = s
        metal.drawableSize = CGSize(width: bounds.width * s, height: bounds.height * s)
        if started {
            player.resize(bounds.size, s)
        } else {
            started = true
            becomeFirstResponder()
            player.start(
                metal, size: bounds.size, scale: s,
                link: CADisplayLink(target: player, selector: #selector(Playback.step(_:))))
        }
    }
    @objc func tap(_ gesture: UITapGestureRecognizer) {
        if gesture.numberOfTouchesRequired == 3 { player.controls.toggle() } else { player.pulse(3) }
    }
    @objc func history(_ pan: UIPanGestureRecognizer) {
        guard pan.state == .changed || pan.state == .ended else { return }
        let y = pan.translation(in: self).y
        let step: CGFloat = 36
        guard abs(y) >= step else { return }
        player.pulse(y < 0 ? 4 : 5)
        pan.setTranslation(CGPoint(x: 0, y: y.truncatingRemainder(dividingBy: step)), in: self)
    }
    @objc func hover(_ gesture: UIHoverGestureRecognizer) {
        let p = gesture.location(in: self)
        let s = layer.contentsScale
        player.invoke { cmvs_apple_pointer($0, p.x * s, p.y * s) }
    }
    @objc func rightClick(_ gesture: UITapGestureRecognizer) {
        let p = gesture.location(in: self)
        let s = layer.contentsScale
        var inside = false
        player.invoke {
            let result = cmvs_apple_pointer($0, p.x * s, p.y * s)
            inside = result == 1
            return result
        }
        if inside { player.pulse(3) }
    }
    private func touchID(_ touch: UITouch) -> UInt64 {
        UInt64(UInt(bitPattern: Unmanaged.passUnretained(touch).toOpaque()))
    }
    private func record(_ touches: Set<UITouch>, _ phase: DebugRecording.Phase) {
        guard let recording = player.debugRecording else { return }
        for touch in touches {
            let id = touchID(touch)
            if phase == .moved || phase == .ended {
                let point = touch.location(in: self)
                let previous = touch.previousLocation(in: self)
                recording.receive(id, phase: phase, contacts: contacts.count,
                                  dx: Int(exactly: (point.x - previous.x).rounded()),
                                  dy: Int(exactly: (point.y - previous.y).rounded()))
            } else {
                recording.receive(id, phase: phase, contacts: contacts.count)
            }
        }
    }
    private func send(_ t: UITouch, _ phase: DebugRecording.Phase, at p: CGPoint) {
        let s = layer.contentsScale
        player.touch(touchID(t), phase: phase, x: p.x * s, y: p.y * s)
    }
    private func discardPending() {
        pendingPress?.cancel()
        pendingPress = nil
        pending = nil
    }
    private func beginPending(_ decision: DebugRecording.Gesture) {
        guard let pending else { return }
        discardPending()
        player.debugRecording?.append(.gesture(decision: decision))
        send(pending.touch, .began, at: pending.origin)
        send(pending.touch, .moved, at: pending.touch.location(in: self))
    }
    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        if touches.allSatisfy({ $0.type == .indirectPointer }), event?.buttonMask.contains(.secondary) == true { return }
        contacts.formUnion(touches)
        record(touches, .began)
        if contacts.count > 1 {
            player.debugRecording?.append(.gesture(decision: .cancelledBySecondContact))
            discardPending()
            blocked = true
            for touch in contacts { send(touch, .cancelled, at: touch.location(in: self)) }
        }
        guard !blocked, let touch = touches.first else { return }
        if player.game.options.gestures && touch.type == .direct {
            // Let a second finger join before the engine samples a primary click.
            player.debugRecording?.append(.gesture(decision: .deferred))
            pending = (touch, touch.location(in: self))
            pendingPress = Task { [weak self] in
                do { try await Task.sleep(for: .milliseconds(100)) } catch { return }
                guard !Task.isCancelled else { return }
                self?.beginPending(.startedOnTimer)
            }
        } else {
            send(touch, .began, at: touch.location(in: self))
        }
    }
    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        record(touches, .moved)
        guard !blocked else { return }
        for touch in touches.intersection(contacts) {
            let point = touch.location(in: self)
            if let pending {
                guard touch == pending.touch else { continue }
                let dx = point.x - pending.origin.x
                let dy = point.y - pending.origin.y
                let dragDistance: CGFloat = 6
                guard dx * dx + dy * dy >= dragDistance * dragDistance else { continue }
                beginPending(.startedOnMovement)
            } else {
                send(touch, .moved, at: point)
            }
        }
    }
    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        record(touches, .ended)
        if !blocked {
            if let pending, touches.contains(pending.touch) { beginPending(.startedOnRelease) }
            for t in touches.intersection(contacts) { send(t, .ended, at: t.location(in: self)) }
        }
        removeContacts(touches)
    }
    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        record(touches, .cancelled)
        if let pending, touches.contains(pending.touch) {
            player.debugRecording?.append(.gesture(decision: .cancelledBySystem))
            discardPending()
        }
        for t in touches { send(t, .cancelled, at: t.location(in: self)) }
        removeContacts(touches)
    }
    private func removeContacts(_ touches: Set<UITouch>) {
        if let recording = player.debugRecording {
            for touch in touches { recording.forget(touchID(touch)) }
        }
        contacts.subtract(touches)
        if contacts.isEmpty { blocked = false }
    }
    private var accepted = [Int: UInt32]()
    private func keys(_ presses: Set<UIPress>, _ down: Bool) -> Set<UIPress> {
        var remaining = presses
        for p in presses {
            guard let key = p.key else { continue }
            let hid = Int(key.keyCode.rawValue)
            if down {
                guard !key.modifierFlags.contains(.command), let code = Self.code(hid) else { continue }
                if !accepted.values.contains(code) { player.key(code, true) }
                accepted[hid] = code
            } else {
                guard let code = accepted.removeValue(forKey: hid) else { continue }
                if !accepted.values.contains(code) { player.key(code, false) }
            }
            remaining.remove(p)
        }
        return remaining
    }
    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        super.pressesBegan(keys(presses, true), with: event)
    }
    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        super.pressesEnded(keys(presses, false), with: event)
    }
    override func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        super.pressesCancelled(keys(presses, false), with: event)
    }

    static func code(_ hid: Int) -> UInt32? {
        switch hid {
        case 4...29: return UInt32(86 + hid - 4)
        case 30...38: return UInt32(77 + hid - 30)
        case 58...69: return UInt32(52 + hid - 58)
        case 104...115: return UInt32(64 + hid - 104)
        case 89...97: return UInt32(38 + hid - 89)
        default:
            return [
                39: 76, 40: 22, 41: 25, 42: 20, 43: 21, 44: 26, 45: 115, 46: 113, 47: 119, 48: 121, 49: 120, 51: 112,
                52: 122, 53: 118, 54: 114, 55: 116, 56: 117, 73: 31, 74: 30, 75: 27, 76: 32, 77: 29, 78: 28, 79: 36,
                80: 35, 81: 34, 82: 33, 84: 51, 85: 47, 86: 49, 87: 48, 88: 22, 98: 37, 99: 50, 224: 24, 225: 23,
                228: 24, 229: 23,
            ][hid]
        }
    }
}

@MainActor final class OrientationDelegate: NSObject, UIApplicationDelegate {
    private static var mask: UIInterfaceOrientationMask = .allButUpsideDown
    func application(_ application: UIApplication, supportedInterfaceOrientationsFor window: UIWindow?)
        -> UIInterfaceOrientationMask
    { (window?.traitCollection.userInterfaceIdiom ?? UIDevice.current.userInterfaceIdiom) == .pad ? .all : Self.mask }
    static func playing(_ value: Bool) {
        guard UIDevice.current.userInterfaceIdiom == .phone else { return }
        mask = value ? .landscape : .allButUpsideDown
        for scene in UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene })
        where scene.activationState == .foregroundActive {
            scene.keyWindow?.rootViewController?.setNeedsUpdateOfSupportedInterfaceOrientations()
            scene.requestGeometryUpdate(.iOS(interfaceOrientations: mask))
        }
    }
}
