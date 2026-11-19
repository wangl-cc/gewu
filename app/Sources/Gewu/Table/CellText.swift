import Foundation

/// Decodes cells packed by Rust: column-major indices, little-endian 64-bit
/// offsets and a validity bitmap that distinguishes null from empty text.
extension CellText {
    /// Bytes retained by this value's buffers.
    var byteCount: Int { utf8Chunks.reduce(0) { $0 + $1.count } + offsets.count + validity.count }

    func cell(row: Int, column: Int) -> String? {
        guard row >= 0, row < Int(nRows), column >= 0, column < Int(nCols) else { return nil }
        let index = column * Int(nRows) + row
        guard validity[index / 8] & (1 << (index % 8)) != 0 else { return nil }
        let (start, end) = offsets.withUnsafeBytes { bytes in
            let offset = index * MemoryLayout<UInt64>.stride
            return (
                Int(UInt64(littleEndian: bytes.loadUnaligned(fromByteOffset: offset, as: UInt64.self))),
                Int(UInt64(littleEndian: bytes.loadUnaligned(fromByteOffset: offset + 8, as: UInt64.self)))
            )
        }
        if start == end { return "" }
        let first = utf8Chunks[0]
        if end <= first.count { return Self.decode(first, start..<end) }

        let chunkSize = first.count
        let firstChunk = start / chunkSize
        let lastChunk = (end - 1) / chunkSize
        if firstChunk == lastChunk {
            let base = firstChunk * chunkSize
            return Self.decode(utf8Chunks[firstChunk], (start - base)..<(end - base))
        }
        // A cell can cross a transport chunk boundary, even inside a UTF-8 scalar.
        var joined = Data(capacity: end - start)
        for index in firstChunk...lastChunk {
            let base = index * chunkSize
            let chunk = utf8Chunks[index]
            joined.append(contentsOf: chunk[max(0, start - base)..<min(chunk.count, end - base)])
        }
        return Self.decode(joined, 0..<joined.count)
    }

    private static func decode(_ data: Data, _ range: Range<Int>) -> String {
        data.withUnsafeBytes { bytes in
            String(decoding: UnsafeRawBufferPointer(rebasing: bytes[range]), as: UTF8.self)
        }
    }
}
