import Foundation
import Testing

/// Tests App table and copy behavior through the real Core bindings.
@Suite
struct TableExecutionTests {
    private static func dataset() throws -> GewuDataset {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("row-loader-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        try "number,label\n1,first\n2,second\n3,third\n".write(to: path, atomically: true, encoding: .utf8)
        return try GewuDataset.open(path: path.path)
    }

    @Test @MainActor
    static func loaderCoalescesReadsAndDeliversUncachedBlocks() async throws {
        let view = try dataset().view()
        let reads = AsyncStream<PendingTableRead>.makeStream()
        let delivered = AsyncStream<RowBlock>.makeStream()
        let loader = TableRowLoader(cache: RowCache(cellLimit: 1)) { view, key in
            try await withCheckedThrowingContinuation { continuation in
                reads.continuation.yield(PendingTableRead(view: view, key: key, continuation: continuation))
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
        #expect(loader.block(for: 1, column: 0) == nil)
        #expect(loader.block(for: 0, column: 1) == nil)
        let first = try #require(await requests.next())
        #expect(first.key == RowCache.Key(start: 0, column: 0))
        first.finish()
        #expect(try #require(await blocks.next()).cells == ["1", "2", "3"])
        #expect(loader.cachedBlock(for: first.key) == nil, "Oversized data is delivered without retaining it")
        let second = try #require(await requests.next())
        #expect(second.key == RowCache.Key(start: 0, column: 1), "Rows in the same block share a request")
        second.finish()
        #expect(try #require(await blocks.next()).cells == ["first", "second", "third"])
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
        let loader = TableRowLoader { view, key in
            try await withCheckedThrowingContinuation { continuation in
                reads.continuation.yield(PendingTableRead(view: view, key: key, continuation: continuation))
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
        _ = loader.block(for: 0, column: 0)
        let oldRead = try #require(await requests.next())
        loader.reset(to: replacement)
        _ = loader.block(for: 0, column: 0)
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
    let key: RowCache.Key
    let continuation: CheckedContinuation<RowBlock, Error>

    func finish() {
        continuation.resume(
            with: Result {
                try view.rows(
                    start: UInt64(key.start), count: UInt32(RowCache.blockSize), columns: key.column.map { [$0] } ?? [])
            })
    }
}
