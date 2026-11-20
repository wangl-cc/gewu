import Foundation
import GewuKit
import SwiftUI

/// A user default with its type and default value, declared once.
struct Preference<Value: Sendable>: Sendable {
    let key: String
    let defaultValue: Value
}

/// Every user default the app reads or writes.
enum Preferences {
    static let tableFontSize = Preference(key: "TableFontSize", defaultValue: 12.0)
    static let tableRowHeight = Preference(key: "TableRowHeight", defaultValue: 28.0)
    static let showsRowNumbers = Preference(key: "ShowRowNumbers", defaultValue: true)
    /// The number shown for the first source row: 0 or 1.
    static let rowNumberBase = Preference(key: "RowNumberBase", defaultValue: 0)
    static let copyIncludesHeaders = Preference(key: "CopyIncludesHeaders", defaultValue: true)
    static let statisticsSignificantDigits = Preference(key: "StatisticsSignificantDigits", defaultValue: 8)
    static let remembersExportFormat = Preference(key: "RememberExportFormat", defaultValue: true)
    static let lastExportFormat = Preference(key: "LastExportFormat", defaultValue: ExportFileFormat.parquet.rawValue)
    static let parquetCompression = Preference(
        key: "ParquetCompression", defaultValue: ParquetCompression.zstd.rawValue)
    static let showsInspector = Preference(key: "ShowColumnInspector", defaultValue: true)
    static let skipsHideColumnWarning = Preference(key: "SkipHideColumnWarning", defaultValue: false)

    /// Valid ranges, applied wherever the values are used.
    static let fontSizes: ClosedRange<Double> = 10...20
    static let rowHeights: ClosedRange<Double> = 24...36
}

extension UserDefaults {
    subscript(preference: Preference<Bool>) -> Bool {
        get { object(forKey: preference.key) as? Bool ?? preference.defaultValue }
        set { set(newValue, forKey: preference.key) }
    }

    subscript(preference: Preference<Int>) -> Int {
        get { object(forKey: preference.key) as? Int ?? preference.defaultValue }
        set { set(newValue, forKey: preference.key) }
    }

    subscript(preference: Preference<String>) -> String {
        get { string(forKey: preference.key) ?? preference.defaultValue }
        set { set(newValue, forKey: preference.key) }
    }
}

extension AppStorage {
    init(_ preference: Preference<Value>) where Value == Bool {
        self.init(wrappedValue: preference.defaultValue, preference.key)
    }

    init(_ preference: Preference<Value>) where Value == Int {
        self.init(wrappedValue: preference.defaultValue, preference.key)
    }

    init(_ preference: Preference<Value>) where Value == Double {
        self.init(wrappedValue: preference.defaultValue, preference.key)
    }
}
