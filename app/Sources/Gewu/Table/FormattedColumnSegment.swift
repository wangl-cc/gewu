import Foundation

/// Decodes a segment formatted by Rust: little-endian 32-bit row offsets and a
/// validity bitmap that distinguishes null from empty text.
extension FormattedColumnSegment {
    /// Bytes retained by this value's buffers.
    var byteCount: Int { utf8.count + offsets.count + validity.count }

    /// Text of a row relative to the segment start; `nil` for null or out of range.
    func cell(row: Int) -> String? {
        guard row >= 0, row < Int(rowCount) else { return nil }
        guard validity[row / 8] & (1 << (row % 8)) != 0 else { return nil }
        let (start, end) = offsets.withUnsafeBytes { bytes in
            let offset = row * MemoryLayout<UInt32>.stride
            return (
                Int(UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self))),
                Int(UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: offset + 4, as: UInt32.self)))
            )
        }
        return utf8.withUnsafeBytes { bytes in
            String(decoding: UnsafeRawBufferPointer(rebasing: bytes[start..<end]), as: UTF8.self)
        }
    }
}
