<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/icon-dark.png">
    <img src="assets/icon-light.png" width="112" height="112" alt="Gewu app icon">
  </picture>
</p>

<h1 align="center">gewu 格物</h1>

<p align="center">A Rust table viewer for large datasets.</p>

Open downloaded data, explore its values and distributions, and find the rows that matter before moving to Python for deeper analysis.

## Features

- Browse, sort, find and filter your data in a native Mac interface.
- Keep a column's statistics and distribution in view while filtering other columns, or explore a sampled scatter plot.
- Switch between XLSX worksheets in the same window.
- Read Parquet, CSV, TSV and XLSX files, including nested Parquet data.
- Copy selected rows or export filtered results. Browsing leaves the source file unchanged.

## Get started

Requires **macOS 15 or later**.

1. Download Gewu from [Releases](https://github.com/wangl-cc/gewu/releases): choose `arm64` for Apple Silicon or `x86_64` for Intel.
2. Unzip the download. You can keep **Gewu.app** in that folder or move it to **Applications**.
3. Launch Gewu and choose a file, or use **File → Open…** (`⌘O`).

The native GPUI migration is being evaluated on this branch. See the [acceptance checklist](docs/dev/native-acceptance.md) for remaining verification.

Use **Gewu → Settings…** (`⌘,`) for display and copy preferences. Adjust histogram bins and scatter point limits beside their charts.

For files without column headers, use **File → Import Options…**. Save the current results with **File → Export…** (`⇧⌘E`); the initial default is Parquet with Zstd compression. See [file formats](docs/spec/file-formats.md) for import and export details.

To build the Rust GPUI application from source, follow the [development guide](DEVELOPMENT.md#build-and-run).

## File formats and limits

Gewu decodes the opened table into memory, so the dataset size you can explore depends on available RAM. Compressed file size can be much smaller than the memory required; for XLSX, only the selected worksheet is loaded as a table.

| Format | Read and export | Main considerations |
| --- | --- | --- |
| Parquet | Typed tables, including lists and structs. | Preferred for preserving types, nested values and nulls. |
| CSV / TSV | Flat tables with inferred column types. | Types are not preserved; empty strings and nulls read as null. Nested values export as display text. |
| XLSX | Read individual worksheets; export one worksheet. | Dates/times and high-precision numbers export as text. Nested values export as display text; workbook formatting and formulas are not preserved. |

See [format details](docs/spec/file-formats.md) for type inference, date handling and conversion rules.

> [!IMPORTANT]
> XLSX worksheets support up to **1,048,576 rows × 16,384 columns**. Gewu reserves one row for headers, allowing **1,048,575 data rows** per export.

## Documentation

- [Development](DEVELOPMENT.md) — Rust and macOS UI build, run and test workflows.
- [Data types](docs/spec/data-types.md) — values, precision and typed input.
- [Find and filtering](docs/spec/search-and-filter.md) — typed cell navigation and filter conditions.
- [File formats](docs/spec/file-formats.md) — reading, exporting and format limits.

## License

Copyright (c) 2026 The gewu contributors.

Gewu is free software: you can redistribute it and/or modify it under the terms of the [GNU General Public License](LICENSE) as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version (`GPL-3.0-or-later`).

Gewu is distributed without any warranty; without even the implied warranty of merchantability or fitness for a particular purpose. See the license for details.

Built-in Catppuccin themes use the [Catppuccin](https://github.com/catppuccin/catppuccin) palette (MIT License, Copyright (c) 2021 Catppuccin).

<!-- markdownlint-disable-file MD033 MD041 -->
