import SwiftUI

struct SettingsView: View {
    @AppStorage("TableFontSize") private var fontSize = 12.0
    @AppStorage("TableRowHeight") private var rowHeight = 28.0
    @AppStorage("ShowRowNumbers") private var showsRowNumbers = true
    @AppStorage("RowNumberBase") private var rowNumberBase = 0
    @AppStorage("CopyIncludesHeaders") private var copyIncludesHeaders = true
    @AppStorage("StatisticsSignificantDigits") private var significantDigits = 8
    @AppStorage("RememberExportFormat") private var rememberExportFormat = true

    var body: some View {
        Form {
            Picker("Text size:", selection: $fontSize) {
                ForEach(10...20, id: \.self) { size in
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
