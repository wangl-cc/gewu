<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="app/Icons/Gewu.icon/Assets/dark.png">
    <img src="app/Icons/Gewu.icon/Assets/light.png" width="128" height="128" alt="Gewu app icon">
  </picture>
</p>

<h1 align="center">gewu 格物</h1>

<p align="center">A fast, native table viewer for large datasets.</p>

Open downloaded data, explore its values and distributions, and find the rows that matter before moving to Python for deeper analysis.

## Features

- Browse, sort, search and filter your data in a native Mac interface.
- Keep a column's statistics and distribution in view while filtering other columns, or explore a sampled scatter plot.
- Switch between XLSX worksheets in the same window.
- Read Parquet, CSV, TSV and XLSX files, including nested Parquet data.
- Copy selected rows or export filtered results. Browsing leaves the source file unchanged.

## Get started

Requires **macOS 15 or later**. Gewu is currently in early development; follow the [build instructions](DEVELOPMENT.md#build-and-run) to build and launch the app.

Use **Gewu → Settings…** (`⌘,`) to adjust table text and row spacing, row numbering, copied headers and statistics precision. Histogram bin counts and scatter point limits are available beside their charts.

CSV, TSV and Excel open with the first row treated as column names. Use **File → Import Options…** to change this for the current file, including when opening fails; reopening resets its search, filters and sorting. Export defaults to Parquet with Zstd compression; choose other formats from the system format menu and other Parquet compression methods in the Export panel. The app remembers the last successful export format unless disabled in Settings.

## Documentation

- [Command line](crates/gewu-cli/README.md) — JSON output for scripts and agents.
- [Development](DEVELOPMENT.md) — building, contributing and code organization.
- [Data types](docs/data-types.md) — values, precision and typed input.
- [Search and filtering](docs/search-and-filter.md) — conditions and result counts.
- [File formats](docs/file-formats.md) — reading, exporting and format limits.

## File formats and limits

| Format | Text | Numbers | Booleans | Dates / times | Lists | Structs |
| --- | :---: | :---: | :---: | :---: | :---: | :---: |
| Parquet | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| CSV / TSV | ✓ | ✓ | ✓ | Limited | ✗ | ✗ |
| XLSX | ✓ | ✓ | ✓ | Limited | ✗ | ✗ |

CSV/TSV recognize ISO-style dates and timestamps, such as `2026-10-30` and `2026-10-30T14:30:00`; locale-specific dates and standalone times remain text. XLSX reads date/time cells, but exports dates, times and high-precision numbers as text. Original date types and display formats are not guaranteed to round-trip through these formats. See [format details](docs/file-formats.md).

> [!IMPORTANT]
> XLSX worksheets support up to **1,048,576 rows × 16,384 columns**. Gewu reserves one row for headers, allowing **1,048,575 data rows** per export.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
