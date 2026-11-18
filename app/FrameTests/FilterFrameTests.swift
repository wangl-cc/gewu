import AppKit
import CoreMedia
import CryptoKit
import ScreenCaptureKit
import SwiftUI
import XCTest

final class FilterFrameTests: XCTestCase {
    @MainActor
    func testFilterPublishesWholeTableFrames() async throws {
        let app = NSApplication.shared
        let policy = app.activationPolicy()
        if policy == .prohibited { app.setActivationPolicy(.accessory) }
        defer { if policy == .prohibited { app.setActivationPolicy(policy) } }
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: url) }
        let lines = (0..<8192).map { "\($0.isMultiple(of: 16) ? "Keep" : "Other"),specimen-\($0),\($0)" }
        try ("cohort,specimen,index\n" + lines.joined(separator: "\n")).write(
            to: url, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: url.path)
        let model = DocumentModel(url: url, dataset: dataset, view: dataset.view())
        defer { model.stop() }

        let viewport = NSRect(x: 40, y: 48, width: 640, height: 400)
        let window = NSWindow(contentRect: viewport, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let host = NSHostingView(rootView: DataTable(model: model, showsRowNumbers: true))
        host.sizingOptions = []
        host.frame = NSRect(origin: .zero, size: viewport.size)
        window.contentView = host
        window.setContentSize(viewport.size)
        window.orderFrontRegardless()
        host.layoutSubtreeIfNeeded()
        try await waitUntil("the table to be installed") { model.table != nil }
        let scroll = try XCTUnwrap(model.table?.view.subviews.compactMap { $0 as? NSScrollView }.first)
        let table = try XCTUnwrap(scroll.documentView as? NSTableView)
        let specimenColumn = try XCTUnwrap(table.tableColumns.firstIndex { $0.identifier.rawValue == "c1" })
        try await waitUntil("the native table to draw its initial rows") {
            window.isVisible && scroll.contentView.bounds.width > 36 && scroll.contentView.bounds.height > 36
                && table.view(atColumn: specimenColumn, row: 1, makeIfNecessary: false) != nil
        }
        window.displayIfNeeded()

        // Keep the compositor sampling even when the table itself stays still.
        // The marker is in the header, outside the sampled rectangle.
        let marker = NSView(frame: .init(x: 1, y: 1, width: 3, height: 3))
        try XCTUnwrap(table.headerView).addSubview(marker)
        marker.wantsLayer = true
        let pulse = Timer.scheduledTimer(withTimeInterval: 1.0 / 60, repeats: true) { _ in
            marker.isHidden.toggle()
        }
        marker.layer?.backgroundColor = NSColor.red.cgColor
        defer { pulse.invalidate() }

        var captured: SCWindow?
        try await waitUntil("the test window to appear in the compositor") {
            let content = try await SCShareableContent.currentProcess
            captured = content.windows.first { Int($0.windowID) == window.windowNumber }
            return captured != nil
        }
        // Capture at one pixel per point and keep overlay scrollers outside the crop.
        let clip = scroll.contentView.convert(scroll.contentView.bounds, to: nil)
        let frameSize = window.frame.size
        let crop = CGRect(x: clip.minX, y: frameSize.height - clip.maxY, width: clip.width, height: clip.height)
            .insetBy(dx: 18, dy: 18).integral
        let config = SCStreamConfiguration()
        config.width = Int(frameSize.width)
        config.height = Int(frameSize.height)
        config.showsCursor = false
        config.ignoreShadowsSingleWindow = true
        config.minimumFrameInterval = CMTime(value: 1, timescale: 120)
        config.pixelFormat = kCVPixelFormatType_32BGRA
        let stream = SCStream(
            filter: SCContentFilter(desktopIndependentWindow: try XCTUnwrap(captured)), configuration: config,
            delegate: nil)
        let frames = TableFrames(crop: crop)
        try stream.addStreamOutput(frames, type: .screen, sampleHandlerQueue: frames.queue)
        try await stream.startCapture()

        do {
            let keep = Filter(condition: .exact(values: ["Keep"], exclude: false), includeNulls: false)
            @MainActor func settle(filtered: Bool) async throws -> SHA256.Digest {
                try await waitUntil("the expected second row after filtering=\(filtered)") {
                    guard !model.isBusy, model.view.rowCount() == (filtered ? 512 : 8192),
                        table.numberOfRows == (filtered ? 512 : 8192),
                        let cell = table.view(atColumn: specimenColumn, row: 1, makeIfNecessary: false)
                            as? NSTableCellView
                    else { return false }
                    return cell.textField?.stringValue == (filtered ? "specimen-16" : "specimen-1")
                }
                let count = frames.samples.count
                try await waitUntil("eight identical visible frames after filtering=\(filtered)") {
                    let recent = frames.samples.dropFirst(count).suffix(8)
                    guard recent.count == 8, let digest = recent.last?.digest else { return false }
                    return recent.allSatisfy { $0.digest == digest }
                }
                return try XCTUnwrap(frames.samples.last?.digest, "The settled table must be visible")
            }

            _ = try await settle(filtered: false)
            model.setFilter(keep, column: 0)
            let subset = try await settle(filtered: true)
            model.clearFilters()
            let full = try await settle(filtered: false)
            XCTAssertNotEqual(full, subset, "The sampled area must distinguish the two row sets")

            var timings = "operation,milliseconds,captured_frames\n"
            for index in 0..<8 {
                let filtered = index.isMultiple(of: 2)
                let start = frames.samples.count
                let submitted = mach_absolute_time()
                model.setFilter(filtered ? keep : nil, column: 0)
                let expected = filtered ? subset : full
                let previous = filtered ? full : subset
                let settled = try await settle(filtered: filtered)
                XCTAssertEqual(settled, expected)
                let transition = frames.samples.dropFirst(start)
                let first = try XCTUnwrap(transition.firstIndex { $0.digest == expected })
                XCTAssertTrue(
                    transition[..<first].allSatisfy { $0.digest == previous }
                        && transition[first...].allSatisfy { $0.digest == expected },
                    "Transition \(index + 1) displayed an incomplete or out-of-order frame"
                )
                let displayed = try XCTUnwrap(transition[first].displayTime)
                XCTAssertGreaterThanOrEqual(displayed, submitted)
                if displayed >= submitted {
                    let milliseconds = CMClockMakeHostTimeFromSystemUnits(displayed - submitted).seconds * 1000
                    timings += "\(filtered ? "apply" : "clear"),\(milliseconds),\(transition.count)\n"
                }
            }
            let attachment = XCTAttachment(string: timings)
            attachment.name = "Filter submission to first complete frame (ms)"
            attachment.lifetime = .keepAlways
            add(attachment)
        } catch {
            try? await stream.stopCapture()
            throw error
        }
        try await stream.stopCapture()
    }

    @MainActor
    private func waitUntil(_ description: String, _ ready: @MainActor () async throws -> Bool) async throws {
        let deadline = ContinuousClock.now.advanced(by: .seconds(10))
        while try await !ready() {
            guard ContinuousClock.now < deadline else {
                throw FrameTimeout(errorDescription: "Timed out waiting for \(description)")
            }
            try await Task.sleep(for: .milliseconds(12))
        }
    }
}

private struct FrameTimeout: LocalizedError {
    let errorDescription: String?
}

/// Keep the pixel digest and compositor timestamp from the same sample.
private final class TableFrames: NSObject, SCStreamOutput {
    struct Frame {
        let digest: SHA256.Digest?
        let displayTime: UInt64?
    }
    let queue = DispatchQueue(label: "gewu.tests.frames")
    let crop: CGRect
    private var captured: [Frame] = []
    var samples: [Frame] { queue.sync { captured } }

    init(crop: CGRect) { self.crop = crop }

    func stream(_ stream: SCStream, didOutputSampleBuffer buffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard
            let attachments = CMSampleBufferGetSampleAttachmentsArray(buffer, createIfNecessary: false)
                as? [[SCStreamFrameInfo: Any]],
            let status = attachments.first?[.status] as? Int
        else { return }
        let time = attachments.first?[.displayTime] as? UInt64
        if status == SCFrameStatus.blank.rawValue {
            captured.append(Frame(digest: nil, displayTime: time))
            return
        }
        guard status == SCFrameStatus.complete.rawValue, let image = buffer.imageBuffer else { return }
        CVPixelBufferLockBaseAddress(image, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(image, .readOnly) }
        guard let pixels = CVPixelBufferGetBaseAddress(image) else { return }
        let bounds = CGRect(x: 0, y: 0, width: CVPixelBufferGetWidth(image), height: CVPixelBufferGetHeight(image))
        guard bounds.contains(crop), !crop.isEmpty else { return }
        var hash = SHA256()
        for y in Int(crop.minY)..<Int(crop.maxY) {
            let row = pixels.advanced(by: y * CVPixelBufferGetBytesPerRow(image) + Int(crop.minX) * 4)
            hash.update(bufferPointer: .init(start: row, count: Int(crop.width) * 4))
        }
        captured.append(Frame(digest: hash.finalize(), displayTime: time))
    }
}
