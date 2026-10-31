import Foundation

/// Core source rows stay zero-based; apply the user's base only for presentation.
func rowNumber(_ sourceRow: UInt64, base: Int) -> UInt64 {
    sourceRow + (base == 1 ? 1 : 0)
}

func formatRowNumber(_ sourceRow: UInt64, base: Int) -> String {
    String(rowNumber(sourceRow, base: base))
}

func formatCount(_ n: UInt64) -> String {
    NumberFormatter.localizedString(from: NSNumber(value: n), number: .decimal)
}
