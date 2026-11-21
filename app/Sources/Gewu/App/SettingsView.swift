import GewuKit
import SwiftUI

struct SettingsView: View {
    @Bindable var updater: AppUpdater
    @AppStorage(Preferences.tableFontSize) private var fontSize: Double
    @AppStorage(Preferences.tableRowHeight) private var rowHeight: Double
    @AppStorage(Preferences.showsRowNumbers) private var showsRowNumbers: Bool
    @AppStorage(Preferences.rowNumberBase) private var rowNumberBase: Int
    @AppStorage(Preferences.copyIncludesHeaders) private var copyIncludesHeaders: Bool
    @AppStorage(Preferences.statisticsSignificantDigits) private var significantDigits: Int
    @AppStorage(Preferences.remembersExportFormat) private var rememberExportFormat: Bool

    var body: some View {
        Form {
            Picker("Text size:", selection: $fontSize) {
                ForEach(Int(Preferences.fontSizes.lowerBound)...Int(Preferences.fontSizes.upperBound), id: \.self) {
                    size in
                    Text("\(size) pt").tag(Double(size))
                }
            }
            Picker("Row spacing:", selection: $rowHeight) {
                Text("Compact").tag(24.0)
                Text("Standard").tag(28.0)
                Text("Relaxed").tag(36.0)
            }
            Toggle("Show row numbers", isOn: $showsRowNumbers)
            Picker("First row number:", selection: $rowNumberBase) {
                Text("0").tag(0)
                Text("1").tag(1)
            }
            .help("Numbers refer to data rows in the source, excluding the header.")
            Divider()
            Toggle("Include column names when copying", isOn: $copyIncludesHeaders)
            Toggle("Remember last export format", isOn: $rememberExportFormat)
            Divider()
            Picker("Statistics precision:", selection: $significantDigits) {
                ForEach(3...15, id: \.self) { digits in
                    Text("\(digits) significant digits").tag(digits)
                }
            }
            .help("Applies to approximate statistics. Original values and exact statistics retain their precision.")
            if updater.updater != nil {
                Divider()
                Toggle("Automatically check for updates", isOn: $updater.automaticallyChecksForUpdates)
                Toggle("Automatically download and install updates", isOn: $updater.automaticallyDownloadsUpdates)
                    .disabled(!updater.automaticallyChecksForUpdates)
                Button("Check for Updates…") { updater.checkForUpdates() }
                    .disabled(!updater.canCheckForUpdates)
            }
        }
        .formStyle(.columns)
        .toggleStyle(.checkbox)
        .pickerStyle(.menu)
        .padding(24)
        .frame(width: 460)
        .fixedSize(horizontal: false, vertical: true)
        .navigationTitle("Gewu Settings")
    }
}
