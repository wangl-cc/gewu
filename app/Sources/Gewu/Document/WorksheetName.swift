/// Core distinguishes worksheet names by UTF-8 bytes, not Unicode canonical equivalence.
struct WorksheetName: Hashable, Sendable {
    let rawValue: String

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.rawValue.utf8.elementsEqual(rhs.rawValue.utf8)
    }

    func hash(into hasher: inout Hasher) {
        let bytes = rawValue.utf8
        hasher.combine(bytes.count)
        for byte in bytes {
            hasher.combine(byte)
        }
    }
}
