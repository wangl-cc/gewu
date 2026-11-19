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

    private static func source(_ block: Int) -> TileKey { TileKey(block: block, column: nil) }
    private static func tile(_ block: Int, _ column: ColumnId) -> TileKey { TileKey(block: block, column: column) }

    @MainActor
    private static func gatedLoader(
        byteLimit: Int = 8 * 1024 * 1024, tileByteLimit: Int = 1024 * 1024
    ) -> (TableRowLoader, AsyncStream<PendingTableRead>, AsyncStream<PendingTableRead>.Continuation) {
        let reads = AsyncStream<PendingTableRead>.makeStream()
        let loader = TableRowLoader(byteLimit: byteLimit, tileByteLimit: tileByteLimit) { view, tiles in
            try await withCheckedThrowingContinuation { continuation in
                reads.continuation.yield(.init(view: view, tiles: tiles, continuation: continuation))
            }
        }
        return (loader, reads.stream, reads.continuation)
    }

    @Test @MainActor
    static func snapshotKeepsCoveredRowsAndReadsOnlyNewColumns() async throws {
        let view = try dataset().unfiltered().unsorted()
        var viewport = TableRowLoader.Viewport(rows: 0..<2, columns: [c0])
        let (loader, stream, reads) = gatedLoader()
        defer { reads.finish() }
        var requests = stream.makeAsyncIterator()
        let pending = Task { try await loader.snapshot(of: view) { viewport } }
        let first = try #require(await requests.next())
        #expect(first.tiles == [source(0), tile(0, c0)])
        viewport = .init(rows: 1..<3, columns: [c0, c2])
        first.finish()
        let second = try #require(await requests.next())
        #expect(second.tiles == [tile(0, c2)])
        second.finish()
        let snapshot = try await pending.value
        #expect(snapshot.viewport == viewport && Set(snapshot.tiles.keys) == [source(0), tile(0, c0), tile(0, c2)])
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
        #expect(next.tiles == [tile(0, c1)])
        next.finish()
    }

    @Test @MainActor
    static func loaderBatchesMissingTilesAndReusesCache() async throws {
        let view = try dataset().unfiltered().unsorted()
        let (loader, stream, reads) = gatedLoader()
        let delivered = AsyncStream<TableTiles>.makeStream()
        defer {
            loader.stop()
            reads.finish()
            delivered.continuation.finish()
        }
        loader.onLoad = { _, tiles in delivered.continuation.yield(tiles) }
        loader.onError = { _, error in Issue.record(error) }
        loader.reset(to: view)
        var requests = stream.makeAsyncIterator()
        var batches = delivered.stream.makeAsyncIterator()
        #expect(loader.tile(for: 0, column: c0) == nil)
        loader.updateViewport(rows: 0..<3, columns: [c0, c1, c0])
        let first = try #require(await requests.next())
        #expect(first.tiles == [source(0), tile(0, c0), tile(0, c1)], "One read, deduplicated columns")
        first.finish()
        let batch = try #require(await batches.next())
        #expect(batch.cells[tile(0, c1)]?.columnTexts(0) == ["first", "second", "third"])
        #expect(batch.sourceRows[0] == [0, 1, 2])
        #expect(loader.tile(for: 1, column: c0).map { $0.text.columnTexts(0) } == ["1", "2", "3"])
        #expect(loader.tile(for: 2, column: c1).map { $0.text.cell(row: $0.row, column: 0) } == "third")
        #expect(loader.tile(for: 3, column: c1) == nil, "Rows past the end are not cached cells")
        #expect(loader.sourceRow(for: 1) == 1)
        #expect(loader.cachedTileCount == 3)
        // Replanning reads only the missing tile; cached cells never enqueue reads.
        loader.updateViewport(rows: 1..<3, columns: [c1, c0, c2])
        let added = try #require(await requests.next())
        #expect(added.tiles == [tile(0, c2)])
        added.finish()
        #expect(try #require(await batches.next()).cells[tile(0, c2)]?.columnTexts(0) == ["10", "20", "30"])
    }

    @Test @MainActor
    static func loaderPrioritizesLatestViewportAndOversizedTilesAffectOnlyThemselves() async throws {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("viewport-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        let long = String(repeating: "x", count: 4096)
        try ("value,long\n" + (0..<300).map { "\($0),\($0 == 200 ? long : "s")" }.joined(separator: "\n"))
            .write(to: path, atomically: true, encoding: .utf8)
        let view = try GewuDataset.open(path: path.path).unfiltered().unsorted()
        let longColumn = ColumnId(raw: 1)
        let (loader, stream, reads) = gatedLoader(tileByteLimit: 2048)
        let delivered = AsyncStream<TableTiles>.makeStream()
        defer {
            loader.stop()
            reads.finish()
            delivered.continuation.finish()
        }
        loader.onLoad = { _, tiles in delivered.continuation.yield(tiles) }
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
        #expect(current.tiles == [source(3), tile(3, c0), tile(3, longColumn)])
        current.finish()
        #expect(try #require(await batches.next()).sourceRows.keys.sorted() == [0])
        let batch = try #require(await batches.next())
        #expect(batch.cells[tile(3, longColumn)]?.cell(row: 8, column: 0) == long, "Delivered for display")
        #expect(loader.tile(for: 200, column: longColumn) == nil, "The oversized tile is not retained")
        #expect(loader.tile(for: 200, column: c0) != nil, "Other tiles in the batch stay cached")
        #expect(loader.sourceRow(for: 200) == 200)
        loader.updateViewport(rows: 192..<202, columns: [c0, longColumn])
        loader.updateViewport(rows: 256..<260, columns: [c0, longColumn])
        let next = try #require(await requests.next())
        #expect(next.tiles.first == source(4), "A completed oversized tile must not start an endless reread loop")
        next.finish()
        #expect(try #require(await batches.next()).sourceRows[4] != nil)
    }

    @Test @MainActor
    static func loaderReadsWideViewportsInOneCallAndClampsRows() async throws {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("wide-viewport-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        let header = (0..<65).map { "c\($0)" }.joined(separator: ",")
        let line = (0..<65).map(String.init).joined(separator: ",")
        try "\(header)\n\(line)\n".write(to: path, atomically: true, encoding: .utf8)
        let view = try GewuDataset.open(path: path.path).unfiltered().unsorted()
        let columns = (0..<65).map { ColumnId(raw: UInt32($0)) }
        let (loader, stream, reads) = gatedLoader()
        let delivered = AsyncStream<TableTiles>.makeStream()
        defer {
            loader.stop()
            reads.finish()
            delivered.continuation.finish()
        }
        loader.onLoad = { _, tiles in delivered.continuation.yield(tiles) }
        loader.onError = { _, error in Issue.record(error) }
        var requests = stream.makeAsyncIterator()
        var batches = delivered.stream.makeAsyncIterator()
        loader.reset(to: view)
        loader.updateViewport(rows: 0..<500, columns: columns)
        let first = try #require(await requests.next())
        #expect(first.tiles == [source(0)] + columns.map { tile(0, $0) })
        first.finish()
        let batch = try #require(await batches.next())
        let last = try #require(batch.cells[tile(0, ColumnId(raw: 64))])
        #expect(last.nRows == 1 && last.nCols == 1 && last.utf8Chunks == [Data("64".utf8)])
        #expect(loader.tile(for: 0, column: ColumnId(raw: 63)).map { $0.text.columnTexts(0) } == ["63"])
        #expect(loader.sourceRow(for: 0) == 0)
        // A source-only viewport also works without formatting any cells.
        loader.reset(to: view)
        loader.updateViewport(rows: 0..<500, columns: [])
        let source = try #require(await requests.next())
        #expect(source.tiles == [Self.source(0)])
        source.finish()
        #expect(try #require(await batches.next()).sourceRows == [0: [0]])
    }

    @Test @MainActor
    static func loaderDiscardsLateReadsAfterReplacingItsView() async throws {
        let dataset = try dataset()
        let filtered = dataset.unfiltered()
        let original = filtered.unsorted()
        let replacement = try filtered.sort(sorts: [ColumnSort(column: c0, descending: true)])
        let (loader, stream, reads) = gatedLoader()
        let delivered = AsyncStream<TableTiles>.makeStream()
        defer {
            loader.stop()
            reads.finish()
            delivered.continuation.finish()
        }
        loader.onLoad = { view, tiles in
            #expect(view === replacement, "Old reads must not update the new table")
            delivered.continuation.yield(tiles)
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
        #expect(try #require(await batches.next()).cells[tile(0, c0)]?.columnTexts(0) == ["3", "2", "1"])
        #expect(loader.sourceRow(for: 0) == 2)
    }

    @Test @MainActor
    static func installedSnapshotReplacesOtherVersions() async throws {
        let dataset = try dataset()
        let filtered = dataset.unfiltered()
        let original = filtered.unsorted()
        let replacement = try filtered.sort(sorts: [ColumnSort(column: c0, descending: true)])
        let loader = TableRowLoader { view, tiles in try TableTiles(reading: view, tiles: tiles) }
        defer { loader.stop() }
        let viewport = TableRowLoader.Viewport(rows: 0..<3, columns: [c0])
        loader.install(try await loader.snapshot(of: original) { viewport })
        #expect(loader.tile(for: 0, column: c0)?.text.cell(row: 0, column: 0) == "1")
        let next = try await loader.snapshot(of: replacement) { viewport }
        #expect(loader.tile(for: 0, column: c0)?.text.cell(row: 0, column: 0) == "1", "Preparing keeps the display")
        loader.install(next)
        #expect(loader.tile(for: 0, column: c0)?.text.cell(row: 0, column: 0) == "3")
        #expect(loader.cachedTileCount == 2, "Tiles of the previous View are released")
    }

    static func text(_ values: [String?], columns: Int = 1) -> CellText {
        let rows = values.count / columns
        var utf8 = Data()
        var offsets: [UInt64] = [0]
        var validity = Data(repeating: 0, count: (values.count + 7) / 8)
        for column in 0..<columns {
            for row in 0..<rows {
                if let value = values[row * columns + column] {
                    utf8.append(contentsOf: value.utf8)
                    let index = column * rows + row
                    validity[index / 8] |= 1 << (index % 8)
                }
                offsets.append(UInt64(utf8.count).littleEndian)
            }
        }
        return CellText(
            nRows: UInt32(rows), nCols: UInt64(columns), utf8Chunks: [utf8],
            offsets: offsets.withUnsafeBytes { Data($0) }, validity: validity)
    }

    @Test
    static func packedTextSurvivesTheBindings() throws {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("row-text-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        let values = ["é", "", "界🦀", "three", "", "five", "six", "seven", "", "\u{FEFF}last"]
        let csv = "number,text\n" + values.enumerated().map { "\($0.offset),\($0.element)" }.joined(separator: "\n")
        try csv.write(to: path, atomically: true, encoding: .utf8)
        let view = try GewuDataset.open(path: path.path).unfiltered().unsorted()
        let block = try view.cellText(start: 0, count: 64, columns: [c1, c0])
        #expect(view.sourceRows(start: 0, count: 64) == (0..<10).map(UInt64.init))
        for (row, value) in values.enumerated() {
            #expect(block.cell(row: row, column: 0) == (value.isEmpty ? nil : value))
            #expect(block.cell(row: row, column: 1) == String(row))
        }
    }

    @Test
    static func packedTextDecodesAcrossTransportChunks() {
        let text = Array("a界bc".utf8)
        let offsets = [0, 1, 4, 6, 6, 6].map { UInt64($0).littleEndian }
        let cells = CellText(
            nRows: 5, nCols: 1, utf8Chunks: [Data(text[0..<3]), Data(text[3..<6])],
            offsets: offsets.withUnsafeBytes { Data($0) },
            validity: Data([0b01111]))
        #expect(cells.texts == ["a", "界", "bc", "", nil])
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
        let pasteboard = NSPasteboard(name: .init("gewu-copy-test-\(UUID())"))
        defer { pasteboard.releaseGlobally() }
        var errors: [Error] = []
        let model = CopyModel(pasteboard: pasteboard) { error, _ in errors.append(error) }
        // A newer copy cancels the unfinished one; only its rows are published.
        model.copy(ordered: view, rows: [0..<4], columns: [c0])
        model.copy(ordered: view, rows: [0..<2, 3..<9], columns: [c1, c0])
        for _ in 0..<200 where model.message == nil { try await Task.sleep(for: .milliseconds(10)) }
        #expect(errors.isEmpty && model.message == "Copied selection")
        let expected = "label\tnumber\nfirst\t1\n\"tab\tvalue\"\t2\nfourth\t4\n"
        #expect(pasteboard.string(forType: .string) == expected)
        #expect(
            pasteboard.data(forType: .init(UTType.tabSeparatedText.identifier)).map {
                String(decoding: $0, as: UTF8.self)
            } == expected)
    }

    @Test
    static func readerReadsSourceAndCellTilesAndHonorsCancellation() async throws {
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("gewu-table-\(UUID()).csv")
        defer { try? FileManager.default.removeItem(at: path) }
        let csv = "value,label\n" + (0..<130).map { "\($0),row\($0)" }.joined(separator: "\n")
        try csv.write(to: path, atomically: true, encoding: .utf8)
        let dataset = try GewuDataset.open(path: path.path)
        let view = dataset.unfiltered().unsorted()
        let reader = TableRowReader()
        let source = try await reader.read(view, tiles: [source(2)])
        #expect(source.cells.isEmpty && source.sourceRows == [2: [128, 129]])
        #expect(try await reader.read(view, tiles: [tile(0, c0)]).cells[tile(0, c0)]?.nRows == 64)
        let work = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return try await reader.read(view, tiles: [tile(0, c0)])
        }
        do {
            _ = try await work.value
            Issue.record("Expected cancelled table read")
        } catch is CancellationError {}
    }
}

extension CellText {
    func columnTexts(_ column: Int) -> [String?] {
        (0..<Int(nRows)).map { cell(row: $0, column: column) }
    }

    var texts: [String?] {
        (0..<Int(nRows)).flatMap { row in (0..<Int(nCols)).map { cell(row: row, column: $0) } }
    }
}

private struct PendingTableRead: Sendable {
    let view: GewuOrderedView
    let tiles: [TileKey]
    let continuation: CheckedContinuation<TableTiles, Error>

    func finish() {
        continuation.resume(with: Result { try TableTiles(reading: view, tiles: tiles) })
    }
}
