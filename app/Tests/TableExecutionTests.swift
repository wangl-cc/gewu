import Foundation
import Testing

/// Tests App table and copy behavior through the real Core bindings.
@Suite
struct TableExecutionTests {
    private static func dataset() throws -> GewuDataset {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("row-loader-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "number,label,other\n1,first,10\n2,second,20\n3,third,30\n".write(
            to: path, atomically: true, encoding: .utf8)
        return try GewuDataset.open(path: path.path)
    }

    @Test @MainActor
    static func snapshotKeepsCoveredRowsAndReadsOnlyNewColumns() async throws {
        let view = try dataset().view()
        var viewport = TableRowLoader.Viewport(rows: 0..<2, columns: [0])
        let reads = AsyncStream<PendingTableRead>.makeStream()
        let loader = TableRowLoader { view, start, columns in
            try await withCheckedThrowingContinuation { continuation in
                reads.continuation.yield(.init(view: view, start: start, columns: columns, continuation: continuation))
            }
        }
        defer { reads.continuation.finish() }
        var requests = reads.stream.makeAsyncIterator()
        let pending = Task { try await loader.snapshot(of: view) { viewport } }
        let first = try #require(await requests.next())
        #expect(first.columns == [0])
        viewport = .init(rows: 1..<3, columns: [0, 2])
        first.finish()
        let second = try #require(await requests.next())
        #expect(second.columns == [2])
        second.finish()
        let snapshot = try await pending.value
        #expect(snapshot.viewport == viewport && snapshot.pages.count == 2)
    }

    @Test @MainActor
    static func failedColumnReadAllowsOtherColumnsInSameView() async throws {
        let view = try dataset().view()
        let reads = AsyncStream<PendingTableRead>.makeStream()
        let loader = TableRowLoader { view, start, columns in
            try await withCheckedThrowingContinuation { continuation in
                reads.continuation.yield(.init(view: view, start: start, columns: columns, continuation: continuation))
            }
        }
        defer {
            loader.stop()
            reads.continuation.finish()
        }
        loader.reset(to: view)
        var requests = reads.stream.makeAsyncIterator()
        loader.updateViewport(rows: 0..<2, columns: [0])
        let first = try #require(await requests.next())
        loader.updateViewport(rows: 0..<2, columns: [0, 1])
        first.continuation.resume(throwing: CocoaError(.fileReadCorruptFile))
        let next = try #require(await requests.next())
        #expect(next.columns == [1])
        next.finish()
    }

    @Test @MainActor
    static func loaderBatchesVisibleColumnsAndReusesCache() async throws {
        let view = try dataset().view()
        let reads = AsyncStream<PendingTableRead>.makeStream()
        let delivered = AsyncStream<RowBlock>.makeStream()
        let loader = TableRowLoader { view, start, columns in
            try await withCheckedThrowingContinuation { continuation in
                reads.continuation.yield(
                    PendingTableRead(view: view, start: start, columns: columns, continuation: continuation))
            }
        }
        defer {
            loader.stop()
            reads.continuation.finish()
            delivered.continuation.finish()
        }
        loader.onLoad = { _, block, _ in delivered.continuation.yield(block) }
        loader.onError = { _, error in Issue.record(error) }
        loader.reset(to: view)
        var requests = reads.stream.makeAsyncIterator()
        var blocks = delivered.stream.makeAsyncIterator()
        #expect(loader.block(for: 0, column: 0) == nil)
        loader.updateViewport(rows: 0..<3, columns: [0, 1, 0])
        let first = try #require(await requests.next())
        #expect(first.start == 0 && first.columns == [0, 1])
        first.finish()
        #expect(try #require(await blocks.next()).cells == ["1", "first", "2", "second", "3", "third"])
        #expect(loader.block(for: 1, column: 0)?.cells == ["1", "2", "3"])
        #expect(loader.block(for: 1, column: 1)?.cells == ["first", "second", "third"])
        #expect(loader.block(for: 1, column: nil)?.sourceRows == [0, 1, 2])
        // Replanning adds only the missing column; cached cells never enqueue reads.
        loader.updateViewport(rows: 1..<3, columns: [1, 0, 2])
        let added = try #require(await requests.next())
        #expect(added.start == 0 && added.columns == [2])
        added.finish()
        #expect(try #require(await blocks.next()).cells == ["10", "20", "30"])
    }

    @Test @MainActor
    static func loaderPrioritizesLatestViewportAndDoesNotRepeatOversizedReads() async throws {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("viewport-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try ("value\n" + (0..<300).map(String.init).joined(separator: "\n"))
            .write(to: path, atomically: true, encoding: .utf8)
        let view = try GewuDataset.open(path: path.path).view()
        let reads = AsyncStream<PendingTableRead>.makeStream()
        let delivered = AsyncStream<RowBlock>.makeStream()
        let loader = TableRowLoader(cache: RowCache(cellLimit: 1)) { view, start, columns in
            try await withCheckedThrowingContinuation { continuation in
                reads.continuation.yield(
                    PendingTableRead(view: view, start: start, columns: columns, continuation: continuation))
            }
        }
        defer {
            loader.stop()
            reads.continuation.finish()
            delivered.continuation.finish()
        }
        loader.onLoad = { _, block, _ in delivered.continuation.yield(block) }
        loader.onError = { _, error in Issue.record(error) }
        var requests = reads.stream.makeAsyncIterator()
        var blocks = delivered.stream.makeAsyncIterator()
        loader.reset(to: view)
        loader.updateViewport(rows: 0..<100, columns: [0])
        let first = try #require(await requests.next())
        #expect(first.start == 0)
        // The block at 64 was demanded, but must not delay this new viewport.
        loader.updateViewport(rows: 192..<200, columns: [0])
        first.finish()
        let current = try #require(await requests.next())
        #expect(current.start == 192)
        current.finish()
        #expect(try #require(await blocks.next()).start == 192)
        #expect(loader.block(for: 192, column: 0) == nil, "Oversized blocks are delivered without retaining them")
        loader.updateViewport(rows: 192..<200, columns: [0])
        loader.updateViewport(rows: 256..<260, columns: [0])
        let next = try #require(await requests.next())
        #expect(next.start == 256, "A completed oversized block must not start an endless reread loop")
        next.finish()
        #expect(try #require(await blocks.next()).start == 256)
    }

    @Test @MainActor
    static func loaderBoundsWideReadsAndClampsRows() async throws {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("wide-viewport-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        let header = (0..<20).map { "c\($0)" }.joined(separator: ",")
        let line = (0..<20).map(String.init).joined(separator: ",")
        try "\(header)\n\(line)\n".write(to: path, atomically: true, encoding: .utf8)
        let view = try GewuDataset.open(path: path.path).view()
        let reads = AsyncStream<PendingTableRead>.makeStream()
        let delivered = AsyncStream<RowBlock>.makeStream()
        let loader = TableRowLoader { view, start, columns in
            try await withCheckedThrowingContinuation { continuation in
                reads.continuation.yield(
                    PendingTableRead(view: view, start: start, columns: columns, continuation: continuation))
            }
        }
        defer {
            loader.stop()
            reads.continuation.finish()
            delivered.continuation.finish()
        }
        loader.onLoad = { _, block, _ in delivered.continuation.yield(block) }
        loader.onError = { _, error in Issue.record(error) }
        var requests = reads.stream.makeAsyncIterator()
        var blocks = delivered.stream.makeAsyncIterator()
        loader.reset(to: view)
        loader.updateViewport(rows: 0..<500, columns: (0..<20).map(UInt32.init))
        let first = try #require(await requests.next())
        #expect(first.columns == (0..<16).map(UInt32.init))
        first.finish()
        #expect(try #require(await blocks.next()).nRows == 1)
        let second = try #require(await requests.next())
        #expect(second.start == 0 && second.columns == (16..<20).map(UInt32.init))
        second.finish()
        #expect(try #require(await blocks.next()).nCols == 4)
        #expect(loader.block(for: 0, column: 19)?.cells == ["19"])
        #expect(loader.block(for: 0, column: nil)?.sourceRows == [0])
        // A source-only viewport also works without formatting any cells.
        loader.reset(to: view)
        loader.updateViewport(rows: 0..<500, columns: [])
        let source = try #require(await requests.next())
        #expect(source.start == 0 && source.columns.isEmpty)
        source.finish()
        #expect(try #require(await blocks.next()).sourceRows == [0])
    }

    @Test @MainActor
    static func loaderDiscardsLateReadsAfterReplacingItsView() async throws {
        let dataset = try dataset()
        let original = dataset.view()
        let replacement = try dataset.query(
            spec: QuerySpec(columns: [ColSpec(column: 0, sort: ColSort(priority: 0, descending: true), filter: nil)]),
            cancellation: Cancellation())
        let reads = AsyncStream<PendingTableRead>.makeStream()
        let delivered = AsyncStream<RowBlock>.makeStream()
        let loader = TableRowLoader { view, start, columns in
            try await withCheckedThrowingContinuation { continuation in
                reads.continuation.yield(
                    PendingTableRead(view: view, start: start, columns: columns, continuation: continuation))
            }
        }
        defer {
            loader.stop()
            reads.continuation.finish()
            delivered.continuation.finish()
        }
        loader.onLoad = { view, block, _ in
            #expect(view === replacement, "Old reads must not update the new table")
            delivered.continuation.yield(block)
        }
        loader.onError = { _, error in Issue.record(error) }
        var requests = reads.stream.makeAsyncIterator()
        var blocks = delivered.stream.makeAsyncIterator()
        loader.reset(to: original)
        loader.updateViewport(rows: 0..<3, columns: [0])
        let oldRead = try #require(await requests.next())
        loader.reset(to: replacement)
        loader.updateViewport(rows: 0..<3, columns: [0])
        let newRead = try #require(await requests.next())
        oldRead.finish()  // Simulate an FFI read completing after cancellation.
        newRead.finish()
        #expect(try #require(await blocks.next()).cells == ["3", "2", "1"])
        #expect(loader.block(for: 0, column: 0)?.sourceRows == [2, 1, 0])
    }

    static func block(_ values: [String?], start: Int = 0, columns: Int = 1) -> RowBlock {
        RowBlock(
            start: UInt64(start), nRows: UInt32(values.count / columns), nCols: UInt32(columns),
            cells: values, sourceRows: (0..<(values.count / columns)).map(UInt64.init))
    }

    @Test
    static func cacheBudgets() throws {
        let cache = RowCache(byteLimit: 256, cellLimit: 3)
        let a = RowCache.Key(start: 0, column: 0)
        let b = RowCache.Key(start: 0, column: 1)
        let c = RowCache.Key(start: 64, column: 0)
        #expect(cache.insert(block(["first", nil]), for: a))
        #expect(cache.insert(block(["second"]), for: b))
        #expect(cache.block(for: a)?.cells == ["first", nil])
        #expect(cache.insert(block(["third"]), for: c))
        #expect(cache.block(for: b) == nil, "Evict least recently used column block")
        #expect(cache.block(for: a) != nil, "Same row block in another column must stay distinct")
        #expect(
            !cache.insert(block([String(repeating: "界", count: 100)]), for: b), "Use UTF-8 bytes, not character count")
        #expect(cache.byteCount <= 256 && cache.cellCount <= 3)
        let metadata = RowBlock(
            start: 0, nRows: 4, nCols: 0, cells: [], sourceRows: [0, 1, 2, 3])
        #expect(!cache.insert(metadata, for: .init(start: 0, column: nil)), "Source-only blocks consume cell budget")
        cache.clear()
        #expect(cache.byteCount == 0 && cache.cellCount == 0 && cache.block(for: a) == nil)
    }

    @Test
    static func copyOrderAndEscaping() throws {
        let columns = Array((0..<130).reversed()).map(UInt32.init)
        let ranges = [2..<4, 100..<165]
        var requests = 0
        let result = try CopyText.format(rows: ranges, columns: columns, names: columns.map { "h\($0)" }) {
            start, count, selected in
            #expect(count <= 64 && selected.count <= 64)
            requests += 1
            return block(
                (start..<(start + count)).flatMap { row in selected.map { "\(row):\($0)" } },
                start: start, columns: selected.count)
        }
        let lines = result.split(separator: "\n")
        #expect(lines.count == 68 && requests == 9)
        #expect(lines[1].split(separator: "\t").first == "2:129")
        #expect(lines.last?.split(separator: "\t").last == "164:0")
        let quoted = try CopyText.format(rows: [0..<1], columns: [0, 1, 2, 3], names: ["a", "b", "c", "d"]) { _, _, _ in
            block(["tab\tvalue", "line\nvalue", "a\"b", nil], columns: 4)
        }
        #expect(quoted == "a\tb\tc\td\n\"tab\tvalue\"\t\"line\nvalue\"\t\"a\"\"b\"\t")
        let withoutHeader = try CopyText.format(
            rows: [0..<2], columns: [0], names: ["value"], includeHeaders: false
        ) { _, _, _ in
            block([nil, "second"])
        }
        #expect(withoutHeader == "\nsecond", "An empty first data row must survive when headers are excluded")
    }

    @Test
    static func copyCancellation() async throws {
        let task = Task.detached {
            withUnsafeCurrentTask { $0?.cancel() }
            return try CopyText.format(rows: [0..<1_000_000], columns: [0], names: ["a"]) { _, _, _ in
                Issue.record("Cancelled copy must not fetch")
                throw CancellationError()
            }
        }
        do {
            _ = try await task.value
            Issue.record("Expected cancellation")
        } catch is CancellationError {}
    }

    @Test
    static func readsAndCopyRetainTheirCapturedView() async throws {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("gewu-table-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        let csv = "value,label\n" + (0..<130).map { "\($0),row\($0)" }.joined(separator: "\n")
        try csv.write(to: path, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: path.path)
        let view = dataset.view()
        let reader = TableRowReader()
        let source = try await reader.read(view, start: 2, count: 4, columns: [])
        #expect(source.cells.isEmpty && source.sourceRows == [2, 3, 4, 5])
        var calls = 0
        let copied = try CopyText.format(rows: [0..<130], columns: [1, 0], names: ["label", "value"]) {
            start, count, columns in
            calls += 1
            if calls == 2 {
                let other = try dataset.query(
                    spec: QuerySpec(columns: [
                        ColSpec(column: 0, sort: ColSort(priority: 0, descending: true), filter: nil)
                    ]),
                    cancellation: Cancellation())
                #expect(try other.rows(start: 0, count: 1, columns: [0]).sourceRows == [129])
            }
            return try view.rows(start: UInt64(start), count: UInt32(count), columns: columns)
        }
        #expect(calls == 3)
        #expect(copied.hasSuffix("row129\t129"))
        #expect(try await reader.read(view, start: 0, count: 1, columns: [0]).sourceRows == [0])
        let work = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return try await reader.read(view, start: 0, count: 64, columns: [0])
        }
        do {
            _ = try await work.value
            Issue.record("Expected cancelled table read")
        } catch is CancellationError {}
    }
}

private struct PendingTableRead: Sendable {
    let view: GewuView
    let start: Int
    let columns: [UInt32]
    let continuation: CheckedContinuation<RowBlock, Error>

    func finish() {
        continuation.resume(
            with: Result {
                try view.rows(
                    start: UInt64(start), count: UInt32(RowCache.blockSize), columns: columns)
            })
    }
}
