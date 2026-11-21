import Foundation
import GewuKit

/// Core source rows stay zero-based; apply the user's base only for presentation.
func rowNumber(_ sourceRow: UInt64, base: Int) -> UInt64 {
    sourceRow + (base == 1 ? 1 : 0)
}

func formatRowNumber(_ sourceRow: UInt64, base: Int) -> String {
    String(rowNumber(sourceRow, base: base))
}

func formatCount<Count: BinaryInteger>(_ n: Count) -> String {
    Int64(clamping: n).formatted(.number)
}

func formatFileSize(_ bytes: UInt64) -> String {
    Int64(clamping: bytes).formatted(.byteCount(style: .file))
}
