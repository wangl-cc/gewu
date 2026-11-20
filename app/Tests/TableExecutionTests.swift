import AppKit
import Foundation
import GewuKit
import Testing
import UniformTypeIdentifiers

/// Tests the table's text cache and copying through the real engine.
@Suite @MainActor
struct TableExecutionTests {
    private static let csv = "number,label,other\n1,first,10\n2,second,20\n3,third,30\n"

    private static func gatedCache(byteLimit: Int = 8 * 1024 * 1024, blockByteLimit: Int = 1024 * 1024)
        -> (TableTextCache, AsyncStream<PendingRead>, AsyncStream<PendingRead>.Continuation)
    {
        let reads = AsyncStream<PendingRead>.makeStream()
        let cache = TableTextCache(byteLimit: byteLimit, blockByteLimit: blockByteLimit) { rows, key in
            try await withCheckedThrowingContinuation { continuation in
                reads.continuation.yield(PendingRead(rows: rows, key: key, continuation: continuation))
            }
        }
        return (cache, reads.stream, reads.continuation)
    }

    private static func delivered(_ cache: TableTextCache) -> AsyncStream<[BlockKey: ColumnText]> {
        let (stream, continuation) = AsyncStream<[BlockKey: ColumnText]>.makeStream()
        cache.onLoad = { continuation.yield($0) }
        cache.onError = { Issue.record($0) }
        return stream
    }

    @Test static func preparingKeepsCoveredRowsAndReadsOnlyNewColumns() async throws {
        let session = try await TestTables.session(csv)
        let (number, other) = (session.source.id("number"), session.source.id("other"))
        var viewport = Viewport(rows: 0..<2, columns: [number])
        let (cache, stream, reads) = gatedCache()
        defer { reads.finish() }
        var requests = stream.makeAsyncIterator()
        let pending = Task { try await cache.prepare(session.initialRows) { viewport } }
        let first = try #require(await requests.next())
        #expect(first.key == BlockKey(block: 0, column: number))
        viewport = Viewport(rows: 1..<3, columns: [number, other])
        await first.finish()
        let second = try #require(await requests.next())
        #expect(second.key == BlockKey(block: 0, column: other))
        await second.finish()
        let prepared = try await pending.value
        #expect(prepared.viewport == viewport)
        cache.install(prepared)
        #expect(cache.text(row: 2, column: other)?.cell(2) == "30")
    }

    @Test static func aFailedBlockDoesNotStopOtherColumns() async throws {
        let session = try await TestTables.session(csv)
        let (number, label) = (session.source.id("number"), session.source.id("label"))
        let (cache, stream, reads) = gatedCache()
        defer {
            cache.stop()
            reads.finish()
        }
        cache.onError = { _ in }
        cache.reset(to: session.initialRows)
        var requests = stream.makeAsyncIterator()
        cache.updateViewport(Viewport(rows: 0..<2, columns: [number]))
        let first = try #require(await requests.next())
        cache.updateViewport(Viewport(rows: 0..<2, columns: [number, label]))
        first.continuation.resume(throwing: CocoaError(.fileReadCorruptFile))
        let next = try #require(await requests.next())
        #expect(next.key == BlockKey(block: 0, column: label))
        await next.finish()
    }

    @Test static func readsOnlyMissingBlocksAndReusesTheCache() async throws {
        let session = try await TestTables.session(csv)
        let (number, label, other) = (
            session.source.id("number"), session.source.id("label"), session.source.id("other")
        )
        let (cache, stream, reads) = gatedCache()
        defer {
            cache.stop()
            reads.finish()
        }
        var batches = delivered(cache).makeAsyncIterator()
        cache.reset(to: session.initialRows)
        var requests = stream.makeAsyncIterator()
        #expect(cache.text(row: 0, column: number) == nil)
        cache.updateViewport(Viewport(rows: 0..<3, columns: [number, label, number]))
        try await #require(await requests.next()).finish()
        try await #require(await requests.next()).finish()
        let batch = try #require(await batches.next())
        #expect(Set(batch.keys) == [BlockKey(block: 0, column: number), BlockKey(block: 0, column: label)])
        #expect(cache.text(row: 2, column: label)?.cell(2) == "third")
        #expect(cache.text(row: 3, column: label) == nil, "Rows past the end are not cached cells")
        #expect(cache.cachedBlockCount == 2)

        cache.updateViewport(Viewport(rows: 1..<3, columns: [label, number, other]))
        let added = try #require(await requests.next())
        #expect(added.key == BlockKey(block: 0, column: other), "Cached blocks never enqueue reads")
        await added.finish()
        #expect(try #require(await batches.next())[BlockKey(block: 0, column: other)]?.cell(0) == "10")
    }

    @Test static func oversizedBlocksStayVisibleWhileOtherColumnsAreCached() async throws {
        let long = String(repeating: "x", count: 4096)
        let session = try await TestTables.session(
            "value,long\n" + (0..<300).map { "\($0),\($0 == 200 ? long : "s")" }.joined(separator: "\n"))
        let (value, longColumn) = (session.source.id("value"), session.source.id("long"))
        let (cache, stream, reads) = gatedCache(blockByteLimit: 2048)
        defer {
            cache.stop()
            reads.finish()
        }
        var batches = delivered(cache).makeAsyncIterator()
        var requests = stream.makeAsyncIterator()
        cache.reset(to: session.initialRows)
        cache.updateViewport(Viewport(rows: 192..<202, columns: [value, longColumn]))
        try await #require(await requests.next()).finish()
        try await #require(await requests.next()).finish()
        _ = try #require(await batches.next())
        #expect(cache.text(row: 200, column: longColumn)?.cell(200) == long, "Kept for display")
        cache.updateViewport(Viewport(rows: 256..<260, columns: [value, longColumn]))
        #expect(cache.text(row: 200, column: longColumn) == nil, "Oversized blocks are released offscreen")
        #expect(cache.text(row: 200, column: value)?.cell(200) == "200", "The small column was cached")
        let next = try #require(await requests.next())
        #expect(next.key == BlockKey(block: 4, column: value), "An oversized block must not start a reread loop")
        await next.finish()
    }

    @Test static func evictionKeepsTheCurrentViewport() async throws {
        let session = try await TestTables.session(
            "a,b\n" + (0..<64).map { "first\($0),second\($0)" }.joined(separator: "\n"))
        let (a, b) = (session.source.id("a"), session.source.id("b"))
        // One block fits in the cache, two do not. Both are visible.
        let cache = TableTextCache(byteLimit: 1024)
        defer { cache.stop() }
        var batches = delivered(cache).makeAsyncIterator()
        cache.reset(to: session.initialRows)
        cache.updateViewport(Viewport(rows: 0..<64, columns: [a]))
        _ = try #require(await batches.next())
        cache.updateViewport(Viewport(rows: 0..<64, columns: [a, b]))
        _ = try #require(await batches.next())
        #expect(cache.text(row: 32, column: a)?.cell(32) == "first32")
        #expect(cache.text(row: 32, column: b)?.cell(32) == "second32")
        cache.updateViewport(Viewport(rows: 0..<64, columns: [b]))
        #expect(cache.text(row: 32, column: a) == nil, "An evicted block is released outside the viewport")
    }

    @Test static func lateReadsForReplacedRowsAreDiscarded() async throws {
        let session = try await TestTables.session(csv)
        let number = session.source.id("number")
        let original = session.initialRows
        let replacement = try await session.sort(original.rowSet, by: [ColumnSort(column: number, descending: true)])
        let (cache, stream, reads) = gatedCache()
        defer {
            cache.stop()
            reads.finish()
        }
        var batches = delivered(cache).makeAsyncIterator()
        var requests = stream.makeAsyncIterator()
        cache.reset(to: original)
        cache.updateViewport(Viewport(rows: 0..<3, columns: [number]))
        let oldRead = try #require(await requests.next())
        cache.reset(to: replacement)
        cache.updateViewport(Viewport(rows: 0..<3, columns: [number]))
        let newRead = try #require(await requests.next())
        await oldRead.finish()  // A read completing after its rows were replaced.
        await newRead.finish()
        #expect(try #require(await batches.next())[BlockKey(block: 0, column: number)]?.cell(0) == "3")
        #expect(cache.text(row: 0, column: number)?.cell(0) == "3")
    }

    @Test static func preparingKeepsTheDisplayUntilInstalled() async throws {
        let session = try await TestTables.session(csv)
        let number = session.source.id("number")
        let original = session.initialRows
        let replacement = try await session.sort(original.rowSet, by: [ColumnSort(column: number, descending: true)])
        let cache = TableTextCache()
        defer { cache.stop() }
        let viewport = Viewport(rows: 0..<3, columns: [number])
        cache.install(try await cache.prepare(original) { viewport })
        #expect(cache.text(row: 0, column: number)?.cell(0) == "1")
        let next = try await cache.prepare(replacement) { viewport }
        #expect(cache.text(row: 0, column: number)?.cell(0) == "1", "Preparing keeps the display")
        cache.install(next)
        #expect(cache.text(row: 0, column: number)?.cell(0) == "3")
        #expect(cache.cachedBlockCount == 1, "Blocks of the previous rows are released")
    }

    @Test static func lruEvictsLeastRecentlyUsedWithinBudgets() {
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

    @Test static func copyPutsTheLatestSelectionOnThePasteboardAsTSV() async throws {
        let session = try await TestTables.session("number,label\n1,first\n2,\"tab\tvalue\"\n3,third\n4,fourth\n")
        let (number, label) = (session.source.id("number"), session.source.id("label"))
        let pasteboard = NSPasteboard(name: .init("gewu-copy-test-\(UUID())"))
        defer { pasteboard.releaseGlobally() }
        let alerts = Alerts()
        let model = CopyModel(alerts: alerts, pasteboard: pasteboard)
        // A newer copy cancels the unfinished one; only its rows are published.
        model.copy(from: session.initialRows, rows: [0..<4], columns: [number])
        model.copy(from: session.initialRows, rows: [0..<2, 3..<9], columns: [label, number])
        await eventually { model.outcome != nil }
        #expect(alerts.current == nil && model.outcome == .copied)
        let expected = "label\tnumber\nfirst\t1\n\"tab\tvalue\"\t2\nfourth\t4"
        #expect(pasteboard.string(forType: .string) == expected)
        #expect(
            pasteboard.data(forType: .init(UTType.tabSeparatedText.identifier)).map {
                String(decoding: $0, as: UTF8.self)
            } == expected)
    }
}

private struct PendingRead: Sendable {
    let rows: DisplayRows
    let key: BlockKey
    let continuation: CheckedContinuation<ColumnText, any Error>

    func finish() async {
        do {
            continuation.resume(returning: try await rows.text(of: key.column, rows: key.rows))
        } catch {
            continuation.resume(throwing: error)
        }
    }
}
