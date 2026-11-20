import AppKit
import Foundation
import Testing
import UniformTypeIdentifiers

/// Tests App table and copy behavior through the real Core bindings.
@Suite
struct TableExecutionTests {
    private static let c0 = ColumnId(raw: 0)
    private static let c1 = ColumnId(raw: 1)
    private static let c2 = ColumnId(raw: 2)

    private static func dataset() throws -> GewuDataset {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("row-loader-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "number,label,other\n1,first,10\n2,second,20\n3,third,30\n".write(
            to: path, atomically: true, encoding: .utf8)
        return try GewuDataset.open(path: path.path)
    }

    private static func key(_ block: Int, _ column: ColumnId) -> BlockKey { BlockKey(block: block, column: column) }

    @MainActor
    private static func gatedLoader(
        byteLimit: Int = 8 * 1024 * 1024, segmentByteLimit: Int = 1024 * 1024
    ) -> (TableSegmentLoader, AsyncStream<PendingTableRead>, AsyncStream<PendingTableRead>.Continuation) {
        let reads = AsyncStream<PendingTableRead>.makeStream()
        let loader = TableSegmentLoader(byteLimit: byteLimit, segmentByteLimit: segmentByteLimit) { view, keys in
            try await withCheckedThrowingContinuation { continuation in
                reads.continuation.yield(.init(view: view, keys: keys, continuation: continuation))
            }
        }
        return (loader, reads.stream, reads.continuation)
    }

    @Test @MainActor
    static func snapshotKeepsCoveredRowsAndReadsOnlyNewColumns() async throws {
        let view = try dataset().unfiltered().unsorted()
        var viewport = TableSegmentLoader.Viewport(rows: 0..<2, columns: [c0])
        let (loader, stream, reads) = gatedLoader()
        defer { reads.finish() }
        var requests = stream.makeAsyncIterator()
        let pending = Task { try await loader.snapshot(of: view) { viewport } }
        let first = try #require(await requests.next())
        #expect(first.keys == [key(0, c0)])
        viewport = .init(rows: 1..<3, columns: [c0, c2])
        first.finish()
        let second = try #require(await requests.next())
        #expect(second.keys == [key(0, c2)])
        second.finish()
        let snapshot = try await pending.value
        #expect(snapshot.viewport == viewport && Set(snapshot.segments.keys) == [key(0, c0), key(0, c2)])
    }

    @Test @MainActor
    static func failedColumnReadAllowsOtherColumnsInSameView() async throws {
        let view = try dataset().unfiltered().unsorted()
        let (loader, stream, reads) = gatedLoader()
        defer {
            loader.stop()
            reads.finish()
        }
        loader.reset(to: view)
        var requests = stream.makeAsyncIterator()
        loader.updateViewport(rows: 0..<2, columns: [c0])
        let first = try #require(await requests.next())
        loader.updateViewport(rows: 0..<2, columns: [c0, c1])
        first.continuation.resume(throwing: CocoaError(.fileReadCorruptFile))
        let next = try #require(await requests.next())
        #expect(next.keys == [key(0, c1)])
        next.finish()
    }

    @Test @MainActor
    static func loaderReadsOnlyMissingSegmentsAndReusesCache() async throws {
        let view = try dataset().unfiltered().unsorted()
        let (loader, stream, reads) = gatedLoader()
        let delivered = AsyncStream<TableSegments>.makeStream()
        defer {
            loader.stop()
            reads.finish()
            delivered.continuation.finish()
        }
        loader.onLoad = { _, loaded in delivered.continuation.yield(loaded) }
        loader.onError = { _, error in Issue.record(error) }
        loader.reset(to: view)
        var requests = stream.makeAsyncIterator()
        var batches = delivered.stream.makeAsyncIterator()
        #expect(loader.segment(for: 0, column: c0) == nil)
        loader.updateViewport(rows: 0..<3, columns: [c0, c1, c0])
        let first = try #require(await requests.next())
        #expect(first.keys == [key(0, c0), key(0, c1)], "One read, deduplicated columns")
        first.finish()
        let batch = try #require(await batches.next())
        #expect(batch.segments[key(0, c1)]?.texts == ["first", "second", "third"])
        #expect(loader.segment(for: 1, column: c0).map { $0.segment.texts } == ["1", "2", "3"])
        #expect(loader.segment(for: 2, column: c1).map { $0.segment.cell(row: $0.row) } == "third")
        #expect(loader.segment(for: 3, column: c1) == nil, "Rows past the end are not cached cells")
        #expect(loader.cachedEntryCount == 2)
        // Replanning reads only the missing segment; cached cells never enqueue reads.
        loader.updateViewport(rows: 1..<3, columns: [c1, c0, c2])
        let added = try #require(await requests.next())
        #expect(added.keys == [key(0, c2)])
        added.finish()
        #expect(try #require(await batches.next()).segments[key(0, c2)]?.texts == ["10", "20", "30"])
    }

    @Test @MainActor
    static func loaderKeepsOversizedSegmentsVisibleAndCachesOtherColumns() async throws {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("viewport-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        let long = String(repeating: "x", count: 4096)
        try ("value,long\n" + (0..<300).map { "\($0),\($0 == 200 ? long : "s")" }.joined(separator: "\n"))
            .write(to: path, atomically: true, encoding: .utf8)
        let view = try GewuDataset.open(path: path.path).unfiltered().unsorted()
        let longColumn = ColumnId(raw: 1)
        let (loader, stream, reads) = gatedLoader(segmentByteLimit: 2048)
        let delivered = AsyncStream<TableSegments>.makeStream()
        defer {
            loader.stop()
            reads.finish()
            delivered.continuation.finish()
        }
        loader.onLoad = { _, loaded in delivered.continuation.yield(loaded) }
        loader.onError = { _, error in Issue.record(error) }
        var requests = stream.makeAsyncIterator()
        var batches = delivered.stream.makeAsyncIterator()
        loader.reset(to: view)
        loader.updateViewport(rows: 0..<10, columns: [c0])
        let first = try #require(await requests.next())
        // Demand moves before the read completes; the next read targets it.
        loader.updateViewport(rows: 192..<202, columns: [c0, longColumn])
        first.finish()
        let current = try #require(await requests.next())
        #expect(current.keys == [key(3, c0), key(3, longColumn)])
        current.finish()
        #expect(try #require(await batches.next()).keys == [key(0, c0)])
        let batch = try #require(await batches.next())
        #expect(batch.segments[key(3, longColumn)]?.cell(row: 8) == long, "Delivered for display")
        #expect(loader.segment(for: 200, column: longColumn)?.segment.cell(row: 8) == long)
        loader.updateViewport(rows: 192..<202, columns: [c0])
        #expect(loader.segment(for: 200, column: c0)?.segment.cell(row: 8) == "200")
        loader.updateViewport(rows: 256..<260, columns: [c0, longColumn])
        #expect(loader.segment(for: 200, column: longColumn) == nil, "Oversized segments are released offscreen")
        #expect(loader.segment(for: 200, column: c0)?.segment.cell(row: 8) == "200", "The small column was cached")
        let next = try #require(await requests.next())
        #expect(
            next.keys == [key(4, c0), key(4, longColumn)],
            "A completed oversized segment must not start an endless reread loop")
        next.finish()
        #expect(try #require(await batches.next()).segments[key(4, c0)] != nil)
    }

    @Test @MainActor
    static func textEvictionKeepsTheCurrentViewport() async throws {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("row-indices-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        let lines = (0..<64).map { "first\($0),second\($0)" }.joined(separator: "\n")
        try ("a,b\n" + lines).write(to: path, atomically: true, encoding: .utf8)
        let view = try GewuDataset.open(path: path.path).unfiltered().unsorted()
        // One column segment fits in the LRU, two do not. Both are visible.
        let loader = TableSegmentLoader(byteLimit: 1024)
        let loaded = AsyncStream<TableSegments>.makeStream()
        loader.onLoad = { _, data in loaded.continuation.yield(data) }
        defer {
            loader.stop()
            loaded.continuation.finish()
        }
        var reads = loaded.stream.makeAsyncIterator()
        loader.reset(to: view)
        loader.updateViewport(rows: 0..<64, columns: [c0])
        _ = try #require(await reads.next())
        loader.updateViewport(rows: 0..<64, columns: [c0, c1])
        let second = try #require(await reads.next())
        #expect(second.keys == [key(0, c1)])
        #expect(loader.segment(for: 32, column: c0)?.segment.cell(row: 32) == "first32")
        #expect(loader.segment(for: 32, column: c1)?.segment.cell(row: 32) == "second32")
        loader.updateViewport(rows: 0..<64, columns: [c1])
        #expect(loader.segment(for: 32, column: c0) == nil, "The evicted segment is released outside the viewport")
    }

    @Test @MainActor
    static func loaderReadsAllColumnsInWideViewportsAndClampsRows() async throws {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("wide-viewport-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        let header = (0..<65).map { "c\($0)" }.joined(separator: ",")
        let line = (0..<65).map(String.init).joined(separator: ",")
        try "\(header)\n\(line)\n".write(to: path, atomically: true, encoding: .utf8)
        let view = try GewuDataset.open(path: path.path).unfiltered().unsorted()
        let columns = (0..<65).map { ColumnId(raw: UInt32($0)) }
        let (loader, stream, reads) = gatedLoader()
        let delivered = AsyncStream<TableSegments>.makeStream()
        defer {
            loader.stop()
            reads.finish()
            delivered.continuation.finish()
        }
        loader.onLoad = { _, loaded in delivered.continuation.yield(loaded) }
        loader.onError = { _, error in Issue.record(error) }
        var requests = stream.makeAsyncIterator()
        var batches = delivered.stream.makeAsyncIterator()
        loader.reset(to: view)
        loader.updateViewport(rows: 0..<500, columns: columns)
        let first = try #require(await requests.next())
        #expect(first.keys == columns.map { key(0, $0) })
        first.finish()
        let batch = try #require(await batches.next())
        #expect(batch.segments.count == 65)
        for (index, column) in columns.enumerated() {
            let segment = try #require(batch.segments[key(0, column)])
            #expect(segment.rowCount == 1 && segment.utf8 == Data(String(index).utf8))
        }
        #expect(loader.segment(for: 0, column: ColumnId(raw: 63)).map { $0.segment.texts } == ["63"])
    }

    @Test @MainActor
    static func loaderDiscardsLateReadsAfterReplacingItsView() async throws {
        let dataset = try dataset()
        let filtered = dataset.unfiltered()
        let original = filtered.unsorted()
        let replacement = try filtered.sort(sorts: [ColumnSort(column: c0, descending: true)])
        let (loader, stream, reads) = gatedLoader()
        let delivered = AsyncStream<TableSegments>.makeStream()
        defer {
            loader.stop()
            reads.finish()
            delivered.continuation.finish()
        }
        loader.onLoad = { view, loaded in
            #expect(view === replacement, "Old reads must not update the new table")
            delivered.continuation.yield(loaded)
        }
        loader.onError = { _, error in Issue.record(error) }
        var requests = stream.makeAsyncIterator()
        var batches = delivered.stream.makeAsyncIterator()
        loader.reset(to: original)
        loader.updateViewport(rows: 0..<3, columns: [c0])
        let oldRead = try #require(await requests.next())
        loader.reset(to: replacement)
        loader.updateViewport(rows: 0..<3, columns: [c0])
        let newRead = try #require(await requests.next())
        oldRead.finish()  // Simulate an FFI read completing after cancellation.
        newRead.finish()
        #expect(try #require(await batches.next()).segments[key(0, c0)]?.texts == ["3", "2", "1"])
    }

    @Test @MainActor
    static func installedSnapshotReplacesOtherVersions() async throws {
        let dataset = try dataset()
        let filtered = dataset.unfiltered()
        let original = filtered.unsorted()
        let replacement = try filtered.sort(sorts: [ColumnSort(column: c0, descending: true)])
        let loader = TableSegmentLoader { view, keys in try TableSegments(reading: view, keys: keys) }
        defer { loader.stop() }
        let viewport = TableSegmentLoader.Viewport(rows: 0..<3, columns: [c0])
        loader.install(try await loader.snapshot(of: original) { viewport })
        #expect(loader.segment(for: 0, column: c0)?.segment.cell(row: 0) == "1")
        let next = try await loader.snapshot(of: replacement) { viewport }
        #expect(loader.segment(for: 0, column: c0)?.segment.cell(row: 0) == "1", "Preparing keeps the display")
        loader.install(next)
        #expect(loader.segment(for: 0, column: c0)?.segment.cell(row: 0) == "3")
        #expect(loader.cachedEntryCount == 1, "Entries of the previous View are released")
    }

    static func segment(_ values: [String?]) -> FormattedColumnSegment {
        var utf8 = Data()
        var offsets: [UInt32] = [0]
        var validity = Data(repeating: 0, count: (values.count + 7) / 8)
        for (row, value) in values.enumerated() {
            if let value {
                utf8.append(contentsOf: value.utf8)
                validity[row / 8] |= 1 << (row % 8)
            }
            offsets.append(UInt32(utf8.count).littleEndian)
        }
        return FormattedColumnSegment(
            rowCount: UInt32(values.count), utf8: utf8, offsets: offsets.withUnsafeBytes { Data($0) },
            validity: validity)
    }

    @Test
    static func segmentsSurviveTheBindings() throws {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("row-text-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        let values = ["é", "", "界🦀", "three", "", "five", "six", "seven", "", "\u{FEFF}last"]
        let csv = "number,text\n" + values.enumerated().map { "\($0.offset),\($0.element)" }.joined(separator: "\n")
        try csv.write(to: path, atomically: true, encoding: .utf8)
        let view = try GewuDataset.open(path: path.path).unfiltered().unsorted()
        let text = try view.formatColumn(column: c1, start: 0, count: 64)
        let numbers = try view.formatColumn(column: c0, start: 0, count: 64)
        #expect(view.sourceRows(start: 0, count: 64) == (0..<10).map(UInt64.init))
        #expect(text.texts == values.map { $0.isEmpty ? nil : $0 })
        #expect(numbers.texts == (0..<10).map { String($0) })
        #expect(text.cell(row: 10) == nil && text.cell(row: -1) == nil)
        #expect(segment(["a", nil, "", "界"]).texts == ["a", nil, "", "界"])
    }

    @Test
    static func lruEvictsLeastRecentlyUsedWithinBudgets() {
        let cache = LRUCache<Int, String>(costLimit: 10, entryCostLimit: 4)
        #expect(cache.insert("a", for: 1, cost: 3))
        #expect(cache.insert("b", for: 2, cost: 3))
        #expect(cache.insert("c", for: 3, cost: 3))
        #expect(cache.value(for: 1) == "a", "Reading marks 1 most recent")
        #expect(cache.insert("d", for: 4, cost: 3))
        #expect(!cache.contains(2) && cache.contains(1) && cache.totalCost == 9)
        #expect(!cache.insert("huge", for: 5, cost: 5), "An entry over its limit is not retained")
        #expect(cache.count == 3, "An oversized entry evicts nothing")
        #expect(!cache.insert("huge", for: 1, cost: 5) && !cache.contains(1), "It still replaces a stale value")
        #expect(cache.insert("e", for: 3, cost: 4) && cache.value(for: 3) == "e" && cache.totalCost == 7)
        for key in 10..<1000 { cache.insert("\(key)", for: key, cost: 1) }
        #expect(cache.count == 10 && cache.totalCost == 10 && cache.value(for: 999) == "999")
        cache.removeAll { $0 % 2 == 0 }
        #expect(cache.count == 5 && cache.totalCost == 5 && cache.value(for: 991) == "991")
        #expect(cache.insert("z", for: 0, cost: 1) && cache.count == 6, "Freed slots are reused")
        cache.removeAll()
        #expect(cache.count == 0 && cache.totalCost == 0 && cache.value(for: 991) == nil)
    }

    @Test @MainActor
    static func copyPutsSelectedRowsOnThePasteboardAsTSV() async throws {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("copy-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "number,label\n1,first\n2,\"tab\tvalue\"\n3,third\n4,fourth\n".write(
            to: path, atomically: true, encoding: .utf8)
        let view = try GewuDataset.open(path: path.path).unfiltered().unsorted()
        let singleCell = try CopyText.format(
            ordered: view, rows: [0..<1], columns: [c1], names: ["label"], header: false, limit: 5)
        #expect(String(decoding: singleCell, as: UTF8.self) == "first")
        let pasteboard = NSPasteboard(name: .init("gewu-copy-test-\(UUID())"))
        defer { pasteboard.releaseGlobally() }
        var errors: [Error] = []
        let model = CopyModel(pasteboard: pasteboard) { error, _ in errors.append(error) }
        // A newer copy cancels the unfinished one; only its rows are published.
        model.copy(ordered: view, rows: [0..<4], columns: [c0], names: ["number"])
        model.copy(ordered: view, rows: [0..<2, 3..<9], columns: [c1, c0], names: ["label", "number"])
        for _ in 0..<200 where model.message == nil { try await Task.sleep(for: .milliseconds(10)) }
        #expect(errors.isEmpty && model.message == "Copied selection")
        let expected = "label\tnumber\nfirst\t1\n\"tab\tvalue\"\t2\nfourth\t4"
        #expect(pasteboard.string(forType: .string) == expected)
        #expect(
            pasteboard.data(forType: .init(UTType.tabSeparatedText.identifier)).map {
                String(decoding: $0, as: UTF8.self)
            } == expected)
    }

    @Test
    static func readerFormatsSegmentsClippedToTheViewAndHonorsCancellation() async throws {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("gewu-table-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        let csv = "value,label\n" + (0..<130).map { "\($0),row\($0)" }.joined(separator: "\n")
        try csv.write(to: path, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: path.path)
        let view = dataset.unfiltered().unsorted()
        let reader = TableSegmentReader()
        let tail = try await reader.read(view, keys: [key(2, c1)])
        #expect(tail.segments[key(2, c1)]?.texts == ["row128", "row129"], "The last block is clipped")
        #expect(try await reader.read(view, keys: [key(0, c0)]).segments[key(0, c0)]?.rowCount == 64)
        let work = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return try await reader.read(view, keys: [key(0, c0)])
        }
        do {
            _ = try await work.value
            Issue.record("Expected cancelled table read")
        } catch is CancellationError {}
    }
}

extension FormattedColumnSegment {
    var texts: [String?] { (0..<Int(rowCount)).map { cell(row: $0) } }
}

private struct PendingTableRead: Sendable {
    let view: GewuOrderedView
    let keys: [BlockKey]
    let continuation: CheckedContinuation<TableSegments, Error>

    func finish() {
        continuation.resume(with: Result { try TableSegments(reading: view, keys: keys) })
    }
}
