import AppKit
import ApplicationServices
import ScreenCaptureKit

// In-process bridge, macOS 15+. The host owns AppKit initialization, consent,
// polling, session binding, selection and persistence. Call on a worker thread.
// No UI, permission requests, logs, file writes, or participant automation here.
private enum SnapshotStatus: Int32, Error {
    case needsScreenRecording = 1, noRemoteShare = 2, selfShare = 3
    case minimized = 4, ambiguous = 5, captureFailure = 6
}

private let teamsBundle = "com.microsoft.teams2"
private let captureLock = NSLock()
private let maxPixels = 1600

// One native SCK request at a time, including across timed-out invocations.
// Only its callback releases this lease. A callback that never arrives leaves
// capture unavailable rather than accumulating requests on subsequent polls.
private final class SCKRequestGate: @unchecked Sendable {
    private let lock = NSLock()
    private var active = false
    func begin() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard !active else { return false }
        active = true
        return true
    }
    func finish() {
        lock.lock()
        active = false
        lock.unlock()
    }
    var isActive: Bool {
        lock.lock()
        defer { lock.unlock() }
        return active
    }
}
private let sckGate = SCKRequestGate()

private struct Deadline {
    let end: UInt64
    init(seconds: Double) {
        end = DispatchTime.now().uptimeNanoseconds + UInt64(seconds * 1_000_000_000)
    }
    func check() throws {
        if DispatchTime.now().uptimeNanoseconds >= end { throw SnapshotStatus.captureFailure }
    }
    var dispatchTime: DispatchTime { DispatchTime(uptimeNanoseconds: end) }
}

// Each AX element gets its own timeout. Setting the application element alone
// does NOT set a timeout on descendants. Never change the host's global timeout.
private func bound(_ element: AXUIElement, _ deadline: Deadline) throws {
    try deadline.check()
    guard AXUIElementSetMessagingTimeout(element, 0.08) == .success else {
        throw SnapshotStatus.captureFailure
    }
}

private func attribute(_ element: AXUIElement, _ name: String,
                       _ deadline: Deadline, optional: Bool = false) throws -> CFTypeRef? {
    try bound(element, deadline)
    var value: CFTypeRef?
    let error = AXUIElementCopyAttributeValue(element, name as CFString, &value)
    try deadline.check()
    if optional && (error == .attributeUnsupported || error == .noValue) { return nil }
    guard error == .success, let value else { throw SnapshotStatus.captureFailure }
    return value
}

private func elements(_ element: AXUIElement, _ name: String, limit: Int,
                      _ deadline: Deadline, optional: Bool = false) throws -> [AXUIElement] {
    try bound(element, deadline)
    var count = 0
    let error = AXUIElementGetAttributeValueCount(element, name as CFString, &count)
    try deadline.check()
    if optional && (error == .attributeUnsupported || error == .noValue) { return [] }
    guard error == .success, count >= 0, count <= limit else { throw SnapshotStatus.captureFailure }
    if count == 0 { return [] }
    var array: CFArray?
    let result = AXUIElementCopyAttributeValues(element, name as CFString, 0, count, &array)
    try deadline.check()
    guard result == .success, let array, CFArrayGetCount(array) == count else {
        throw SnapshotStatus.captureFailure
    }
    var afterCount = 0
    guard AXUIElementGetAttributeValueCount(element, name as CFString, &afterCount) == .success,
          afterCount == count else { throw SnapshotStatus.captureFailure }
    try deadline.check()
    let values = array as [AnyObject]
    guard values.allSatisfy({ CFGetTypeID($0) == AXUIElementGetTypeID() }) else {
        throw SnapshotStatus.captureFailure
    }
    return values.map { $0 as! AXUIElement }
}

private struct Node {
    let role: String
    let title: String
    let description: String
    let busy: Bool
}

private func node(_ element: AXUIElement, _ deadline: Deadline) throws -> Node {
    try bound(element, deadline)
    let names = [kAXRoleAttribute, kAXTitleAttribute, kAXDescriptionAttribute, "AXElementBusy"]
    var array: CFArray?
    let error = AXUIElementCopyMultipleAttributeValues(element, names as CFArray, [], &array)
    try deadline.check()
    guard error == .success, let values = array as? [AnyObject], values.count == names.count else {
        throw SnapshotStatus.captureFailure
    }
    func value(_ index: Int) throws -> AnyObject? {
        let v = values[index]
        if CFGetTypeID(v) == AXValueGetTypeID() {
            let ax = v as! AXValue
            var error = AXError.failure
            guard AXValueGetType(ax) == .axError, AXValueGetValue(ax, .axError, &error),
                  index != 0, error == .attributeUnsupported || error == .noValue else {
                throw SnapshotStatus.captureFailure
            }
            return nil
        }
        // A null value is not proof of a successful traversal.
        guard CFGetTypeID(v) != CFNullGetTypeID() else { throw SnapshotStatus.captureFailure }
        return v
    }
    func text(_ index: Int) throws -> String {
        guard let v = try value(index) else { return "" }
        guard let s = v as? String, s.utf8.count <= 16_384 else { throw SnapshotStatus.captureFailure }
        return s
    }
    var busy = false
    if let v = try value(3) {
        guard CFGetTypeID(v) == CFBooleanGetTypeID() || CFGetTypeID(v) == CFNumberGetTypeID(),
              let n = v as? NSNumber else { throw SnapshotStatus.captureFailure }
        busy = n.boolValue
    }
    return try Node(role: text(0), title: text(1), description: text(2), busy: busy)
}

private func rect(_ element: AXUIElement, _ deadline: Deadline) throws -> CGRect {
    guard let pv = try attribute(element, kAXPositionAttribute, deadline),
          let sv = try attribute(element, kAXSizeAttribute, deadline),
          CFGetTypeID(pv) == AXValueGetTypeID(), CFGetTypeID(sv) == AXValueGetTypeID() else {
        throw SnapshotStatus.captureFailure
    }
    var p = CGPoint.zero, s = CGSize.zero
    guard AXValueGetValue(pv as! AXValue, .cgPoint, &p),
          AXValueGetValue(sv as! AXValue, .cgSize, &s),
          p.x.isFinite, p.y.isFinite, s.width.isFinite, s.height.isFinite,
          s.width > 0, s.height > 0, s.width <= 32768, s.height <= 32768 else {
        throw SnapshotStatus.captureFailure
    }
    return CGRect(origin: p, size: s)
}

private struct TeamsState {
    let pid: pid_t
    let window: AXUIElement
    let title: String
    let frame: CGRect
    let stage: CGRect
    let presenter: String // Private and transient. Never exported or logged.
    func same(as other: TeamsState) -> Bool {
        pid == other.pid && CFEqual(window, other.window) && title == other.title &&
        frame == other.frame && stage == other.stage && presenter == other.presenter
    }
}

private func state() throws -> TeamsState {
    guard AXIsProcessTrusted() else { throw SnapshotStatus.captureFailure }
    let apps = NSRunningApplication.runningApplications(withBundleIdentifier: teamsBundle)
        .filter { !$0.isTerminated }
    guard apps.count <= 1 else { throw SnapshotStatus.ambiguous }
    guard let app = apps.first else { throw SnapshotStatus.noRemoteShare }
    let deadline = Deadline(seconds: 1.5)
    let root = AXUIElementCreateApplication(app.processIdentifier)
    let windows = try elements(root, kAXWindowsAttribute, limit: 32, deadline)
    var windowNodes: [Node] = []
    for window in windows { windowNodes.append(try node(window, deadline)) }
    if windowNodes.contains(where: { $0.title == "Sharing control bar" }) {
        throw SnapshotStatus.selfShare
    }
    var candidates: [TeamsState] = []
    var minimizedRemote = false
    var visited = 0
    for (index, window) in windows.enumerated() {
        guard windowNodes[index].role == kAXWindowRole else { throw SnapshotStatus.captureFailure }
        guard let minimizedValue = try attribute(window, kAXMinimizedAttribute, deadline),
              CFGetTypeID(minimizedValue) == CFBooleanGetTypeID(),
              let minimized = minimizedValue as? Bool else { throw SnapshotStatus.captureFailure }
        var stack: [(AXUIElement, Int)] = [(window, 0)]
        var share = false, leave = false, stop = false, loading = false
        var remote: [(AXUIElement, String)] = []
        while let (element, depth) = stack.popLast() {
            visited += 1
            guard visited <= 8192, depth <= 64 else { throw SnapshotStatus.captureFailure }
            let n = try node(element, deadline)
            let labels = [n.title, n.description]
            if n.role == kAXButtonRole {
                share = share || labels.contains("Share")
                leave = leave || labels.contains("Leave")
                stop = stop || labels.contains("Stop sharing")
            }
            loading = loading || n.busy || labels.contains { $0.hasPrefix("Loading content") }
            if n.role == kAXMenuItemRole, n.title.hasPrefix("Content shared by "),
               !n.title.dropFirst("Content shared by ".count).trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                remote.append((element, n.title))
            }
            let children = try elements(element, kAXChildrenAttribute,
                                        limit: 8192 - visited, deadline, optional: true)
            guard stack.count + children.count + visited <= 8192 else { throw SnapshotStatus.captureFailure }
            stack.append(contentsOf: children.map { ($0, depth + 1) })
        }
        // A self-share in any joined/remote/compact meeting vetoes the entire sample.
        if stop { throw SnapshotStatus.selfShare }
        // Exact English controls establish the supported UI language. User mic
        // and camera state are deliberately irrelevant. Unknown languages refuse.
        guard share && leave, !remote.isEmpty else { continue }
        guard remote.count == 1 else { throw SnapshotStatus.ambiguous }
        if minimized { minimizedRemote = true; continue }
        guard !loading else { throw SnapshotStatus.noRemoteShare }
        guard let language = try attribute(remote[0].0, "AXLanguage", deadline, optional: true) as? String,
              language.lowercased().replacingOccurrences(of: "_", with: "-").split(separator: "-").first == "en" else {
            throw SnapshotStatus.noRemoteShare
        }
        let frame = try rect(window, deadline), stage = try rect(remote[0].0, deadline)
        guard frame.contains(stage), stage.width >= 2, stage.height >= 2 else {
            throw SnapshotStatus.captureFailure
        }
        candidates.append(TeamsState(pid: app.processIdentifier, window: window,
                                     title: windowNodes[index].title, frame: frame,
                                     stage: stage, presenter: remote[0].1))
    }
    guard candidates.count <= 1, !(minimizedRemote && !candidates.isEmpty) else {
        throw SnapshotStatus.ambiguous
    }
    guard let result = candidates.first else {
        throw minimizedRemote ? SnapshotStatus.minimized : SnapshotStatus.noRemoteShare
    }
    try deadline.check()
    return result
}

// The callback owns only this box. A late callback never touches the caller's
// struct, nor a stack variable that has gone away, and drops a timed-out image.
private final class ResultBox<T>: @unchecked Sendable {
    private let lock = NSLock()
    private let semaphore = DispatchSemaphore(value: 0)
    private var result: Result<T, Error>?
    private var expired = false
    func finish(_ result: Result<T, Error>) {
        lock.lock()
        if !expired { self.result = result }
        lock.unlock()
        semaphore.signal()
    }
    func wait(_ deadline: Deadline) throws -> T {
        let completed = semaphore.wait(timeout: deadline.dispatchTime) == .success
        lock.lock()
        defer { lock.unlock() }
        expired = true
        guard completed, let result else { self.result = nil; throw SnapshotStatus.captureFailure }
        self.result = nil
        return try result.get()
    }
}

private func content(_ deadline: Deadline) throws -> SCShareableContent {
    let box = ResultBox<SCShareableContent>()
    try deadline.check()
    guard sckGate.begin() else { throw SnapshotStatus.captureFailure }
    SCShareableContent.getExcludingDesktopWindows(true, onScreenWindowsOnly: false) { value, error in
        sckGate.finish()
        if let value, error == nil { box.finish(.success(value)) }
        else { box.finish(.failure(SnapshotStatus.captureFailure)) }
    }
    return try box.wait(deadline)
}

private func matches(_ window: SCWindow, _ state: TeamsState) -> Bool {
    window.owningApplication?.processID == state.pid &&
    window.owningApplication?.bundleIdentifier == teamsBundle &&
    window.title == state.title && window.frame == state.frame && window.isOnScreen
}

private func image(_ window: SCWindow, _ state: TeamsState, _ deadline: Deadline) throws -> CGImage {
    let filter = SCContentFilter(desktopIndependentWindow: window)
    let size = filter.contentRect.size
    guard abs(size.width - state.frame.width) <= 0.5,
          abs(size.height - state.frame.height) <= 0.5,
          filter.pointPixelScale.isFinite, filter.pointPixelScale > 0 else {
        throw SnapshotStatus.captureFailure
    }
    let nativeWidth = size.width * CGFloat(filter.pointPixelScale)
    let nativeHeight = size.height * CGFloat(filter.pointPixelScale)
    let scale = min(1, CGFloat(maxPixels) / max(nativeWidth, nativeHeight))
    let config = SCStreamConfiguration()
    config.width = max(1, Int((nativeWidth * scale).rounded(.down)))
    config.height = max(1, Int((nativeHeight * scale).rounded(.down)))
    config.preservesAspectRatio = true
    config.scalesToFit = false
    config.capturesAudio = false
    config.captureMicrophone = false
    config.showsCursor = false
    config.showMouseClicks = false
    config.ignoreShadowsSingleWindow = true
    config.includeChildWindows = false
    // sourceRect is intentionally unused for independent-window capture.
    let box = ResultBox<CGImage>()
    try deadline.check()
    guard sckGate.begin() else { throw SnapshotStatus.captureFailure }
    SCScreenshotManager.captureImage(contentFilter: filter, configuration: config) { value, error in
        sckGate.finish()
        if let value, error == nil { box.finish(.success(value)) }
        else { box.finish(.failure(SnapshotStatus.captureFailure)) }
    }
    let result = try box.wait(deadline)
    guard result.width == config.width, result.height == config.height,
          result.width <= maxPixels, result.height <= maxPixels else {
        throw SnapshotStatus.captureFailure
    }
    return result
}

private func fingerprint(_ state: TeamsState, _ windowID: CGWindowID) -> UInt64 {
    var hash: UInt64 = 14695981039346656037
    func byte(_ b: UInt8) { hash = (hash ^ UInt64(b)) &* 1099511628211 }
    func number(_ n: UInt64) { for shift in stride(from: 0, to: 64, by: 8) { byte(UInt8(truncatingIfNeeded: n >> shift)) } }
    number(UInt64(UInt32(bitPattern: state.pid)))
    number(UInt64(windowID))
    number(UInt64(state.presenter.utf8.count))
    for b in state.presenter.utf8 { byte(b) }
    for r in [state.frame, state.stage] {
        for value in [r.minX, r.minY, r.width, r.height] { number(Double(value == 0 ? 0 : value).bitPattern) }
    }
    return hash
}

private func windowIdentity(_ state: TeamsState, _ windowID: CGWindowID) -> UInt64 {
    var hash: UInt64 = 14695981039346656037
    func byte(_ b: UInt8) { hash = (hash ^ UInt64(b)) &* 1099511628211 }
    for n in [UInt64(UInt32(bitPattern: state.pid)), UInt64(windowID), UInt64(state.title.utf8.count)] {
        for shift in stride(from: 0, to: 64, by: 8) { byte(UInt8(truncatingIfNeeded: n >> shift)) }
    }
    for b in state.title.utf8 { byte(b) }
    return hash
}

private func pixels(_ image: CGImage, _ state: TeamsState) throws -> (UnsafeMutablePointer<UInt8>, Int, Int) {
    let sx = CGFloat(image.width) / state.frame.width, sy = CGFloat(image.height) / state.frame.height
    let r = state.stage.offsetBy(dx: -state.frame.minX, dy: -state.frame.minY)
    // Round inward so the exported pixels never include an adjacent pane.
    let left = ceil(r.minX * sx), top = ceil(r.minY * sy)
    let right = floor(r.maxX * sx), bottom = floor(r.maxY * sy)
    let crop = CGRect(x: left, y: top, width: right - left, height: bottom - top)
    guard crop.width >= 1, crop.height >= 1,
          CGRect(x: 0, y: 0, width: image.width, height: image.height).contains(crop),
          let stage = image.cropping(to: crop) else { throw SnapshotStatus.captureFailure }
    let width = stage.width, height = stage.height
    guard width <= maxPixels, height <= maxPixels,
          let memory = malloc(width * height * 4) else { throw SnapshotStatus.captureFailure }
    let buffer = memory.assumingMemoryBound(to: UInt8.self)
    guard let color = CGColorSpace(name: CGColorSpace.sRGB),
          let context = CGContext(data: memory, width: width, height: height,
                                  bitsPerComponent: 8, bytesPerRow: width * 4, space: color,
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else {
        free(memory)
        throw SnapshotStatus.captureFailure
    }
    context.setBlendMode(.copy)
    context.draw(stage, in: CGRect(x: 0, y: 0, width: width, height: height))
    // Return conventional straight RGBA even if SCK supplies translucent pixels.
    for i in stride(from: 0, to: width * height * 4, by: 4) {
        let alpha = Int(buffer[i + 3])
        if alpha == 0 { buffer[i] = 0; buffer[i + 1] = 0; buffer[i + 2] = 0 }
        else if alpha < 255 {
            for channel in 0..<3 { buffer[i + channel] = UInt8(min(255, (Int(buffer[i + channel]) * 255 + alpha / 2) / alpha)) }
        }
    }
    return (buffer, width, height)
}

@_cdecl("hex_teams_snapshot_capture")
public func hexTeamsSnapshotCapture(_ output: UnsafeMutablePointer<HexTeamsSnapshot>?) {
    guard let output else { return }
    // Caller must free a previous result before reusing this struct.
    output.pointee = HexTeamsSnapshot()
    output.pointee.status = SnapshotStatus.captureFailure.rawValue
    guard !Thread.isMainThread, captureLock.try() else { return }
    defer { captureLock.unlock() }
    guard !sckGate.isActive else { return }
    autoreleasepool {
        do {
            guard CGPreflightScreenCaptureAccess() else { throw SnapshotStatus.needsScreenRecording }
            let before = try state()
            // One shared two-second budget for all asynchronous SCK callbacks.
            let deadline = Deadline(seconds: 2)
            let windows = try content(deadline).windows.filter { matches($0, before) }
            guard windows.count <= 1 else { throw SnapshotStatus.ambiguous }
            guard let window = windows.first else { throw SnapshotStatus.captureFailure }
            let captured = try image(window, before, deadline)
            guard try state().same(as: before) else { throw SnapshotStatus.captureFailure }
            let after = try content(deadline).windows.filter { matches($0, before) }
            guard after.count == 1, after[0].windowID == window.windowID else { throw SnapshotStatus.captureFailure }
            let (rgba, width, height) = try pixels(captured, before)
            var transferred = false
            defer { if !transferred { free(rgba) } }
            // Revalidate after conversion, immediately before ownership transfer.
            guard CGPreflightScreenCaptureAccess(), try state().same(as: before) else { throw SnapshotStatus.captureFailure }
            output.pointee.width = UInt32(width)
            output.pointee.height = UInt32(height)
            output.pointee.window = window.windowID
            output.pointee.source = fingerprint(before, window.windowID)
            output.pointee.rgba = rgba
            output.pointee.length = width * height * 4
            output.pointee.window_identity = windowIdentity(before, window.windowID)
            output.pointee.status = 0
            transferred = true
        } catch {
            output.pointee.status = (error as? SnapshotStatus ?? .captureFailure).rawValue
        }
    }
}

@_cdecl("hex_teams_snapshot_free")
public func hexTeamsSnapshotFree(_ pixels: UnsafeMutablePointer<UInt8>?) {
    free(pixels)
}
