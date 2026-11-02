// probe: startup and input latency for terminal emulators on macOS.
//
//   probe startup <owner> <runs> -- <command…>   time until the owner's new window is on screen
//   probe latency <owner> <samples>              key press to changed pixels, typing into `cat`
//
// The latency probe needs Accessibility (to post keys) and Screen Recording.

import AppKit
import CoreMedia
import Foundation
import ScreenCaptureKit

func now() -> Double { CACurrentMediaTime() }

func windows(_ owner: String) -> [(id: CGWindowID, frame: CGRect)] {
    let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
    return list.compactMap { w in
        guard (w[kCGWindowOwnerName as String] as? String) == owner,
              (w[kCGWindowLayer as String] as? Int) == 0,
              let b = w[kCGWindowBounds as String] as? [String: Double],
              let h = b["Height"], h > 150,
              let id = w[kCGWindowNumber as String] as? CGWindowID else { return nil }
        return (id, CGRect(x: b["X"]!, y: b["Y"]!, width: b["Width"]!, height: h))
    }
}

func startup(owner: String, runs: Int, command: [String]) {
    var times: [Double] = []
    for _ in 0..<runs {
        let before = Set(windows(owner).map(\.id))
        let process = Process()
        process.executableURL = URL(fileURLWithPath: command[0])
        process.arguments = Array(command.dropFirst())
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        let t0 = now()
        try! process.run()
        var seen: Double? = nil
        while now() - t0 < 10 {
            if windows(owner).contains(where: { !before.contains($0.id) }) { seen = now() - t0; break }
            usleep(1000)
        }
        if let seen { times.append(seen * 1000) }
        // Only the process this probe started is closed; anything else the user runs stays.
        process.terminate()
        let deadline = now() + 5
        while process.isRunning && now() < deadline { usleep(20_000) }
        if process.isRunning { kill(process.processIdentifier, SIGKILL) }
        while !windows(owner).filter({ !before.contains($0.id) }).isEmpty && now() < deadline + 3 { usleep(20_000) }
        usleep(400_000)
    }
    report(times)
}

func report(_ samples: [Double]) {
    let s = samples.sorted()
    guard !s.isEmpty else { print("no samples"); return }
    let median = s[s.count / 2]
    let p90 = s[min(s.count - 1, Int(Double(s.count) * 0.9))]
    print(String(format: "n=%d median=%.1f ms p90=%.1f ms min=%.1f ms", s.count, median, p90, s.first!))
}

final class Watcher: NSObject, SCStreamOutput {
    var last: UInt64 = 0
    var lastAt: Double = 0
    var onChange: ((Double) -> Void)?

    func stream(_ stream: SCStream, didOutputSampleBuffer buffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen,
              let attachments = CMSampleBufferGetSampleAttachmentsArray(buffer, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
              let raw = attachments.first?[.status] as? Int, SCFrameStatus(rawValue: raw) == .complete,
              let pixels = CMSampleBufferGetImageBuffer(buffer) else { return }
        CVPixelBufferLockBaseAddress(pixels, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(pixels, .readOnly) }
        let base = CVPixelBufferGetBaseAddress(pixels)!.assumingMemoryBound(to: UInt8.self)
        let n = CVPixelBufferGetBytesPerRow(pixels) * CVPixelBufferGetHeight(pixels)
        var hash: UInt64 = 1469598103934665603
        for i in stride(from: 0, to: n, by: 4) { hash = (hash ^ UInt64(base[i] &+ base[i + 1] &+ base[i + 2])) &* 1099511628211 }
        let at = CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(buffer))
        if hash != last {
            last = hash
            lastAt = at
            onChange?(at)
        }
    }
}

func key(_ code: CGKeyCode) {
    let src = CGEventSource(stateID: .hidSystemState)
    CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: true)?.post(tap: .cghidEventTap)
    CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: false)?.post(tap: .cghidEventTap)
}

func hostNow() -> Double { CMTimeGetSeconds(CMClockGetTime(CMClockGetHostTimeClock())) }

func latency(owner: String, samples: Int) async {
    guard let target = windows(owner).first else { print("no \(owner) window"); return }
    let content = try! await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
    guard let window = content.windows.first(where: { $0.windowID == target.id }) else { print("window not shareable"); return }
    let config = SCStreamConfiguration()
    let strip = CGRect(x: 0, y: 24, width: min(420, window.frame.width), height: 90)
    config.sourceRect = strip
    config.width = Int(strip.width * 2)
    config.height = Int(strip.height * 2)
    config.minimumFrameInterval = CMTime(value: 1, timescale: 240)
    config.queueDepth = 6
    config.showsCursor = false
    let watcher = Watcher()
    let stream = SCStream(filter: SCContentFilter(desktopIndependentWindow: window), configuration: config, delegate: nil)
    try! stream.addStreamOutput(watcher, type: .screen, sampleHandlerQueue: DispatchQueue(label: "frames"))
    try! await stream.startCapture()
    let app = NSRunningApplication(processIdentifier: window.owningApplication!.processID)
    var times: [Double] = []
    var skipped = 0
    var missed = 0
    for _ in 0..<samples {
        app?.activate()
        try? await Task.sleep(nanoseconds: 250_000_000)
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == window.owningApplication!.processID else {
            skipped += 1
            continue
        }
        let done = DispatchSemaphore(value: 0)
        var seen: Double? = nil
        let t0 = hostNow()
        watcher.onChange = { at in
            if seen == nil && at > t0 { seen = at; done.signal() }
        }
        key(0)
        _ = done.wait(timeout: .now() + 1)
        watcher.onChange = nil
        if let seen { times.append((seen - t0) * 1000) } else { missed += 1 }
        key(51)
        try? await Task.sleep(nanoseconds: 200_000_000)
    }
    try? await stream.stopCapture()
    if skipped > 0 { print("skipped \(skipped) samples: another app was in front") }
    if missed > 0 { print("missed \(missed) samples: no change within 1 s") }
    if ProcessInfo.processInfo.environment["PROBE_VERBOSE"] != nil { print(times.map { String(format: "%.1f", $0) }.joined(separator: " ")) }
    report(times)
}

let args = Array(CommandLine.arguments.dropFirst())
switch args.first {
case "startup":
    let split = args.firstIndex(of: "--")!
    startup(owner: args[1], runs: Int(args[2])!, command: Array(args[(split + 1)...]))
case "latency":
    let sem = DispatchSemaphore(value: 0)
    Task { await latency(owner: args[1], samples: Int(args[2])!); sem.signal() }
    sem.wait()
default:
    print("probe startup <owner> <runs> -- <command…> | probe latency <owner> <samples>")
}
