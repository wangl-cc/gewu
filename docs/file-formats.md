# File formats

Gewu reads and exports Parquet, CSV, TSV and XLSX. Extensions are recognized case-insensitively, with a Parquet magic fallback for unconventional filenames. Browsing leaves the source file unchanged; export writes the current result rows and selected column order.

The whole file is decoded into memory when opened. Compressed file size does not indicate the memory needed to explore it. Format support does not guarantee lossless preservation of types: see [data types](data-types.md) for the behavior of a column after loading.

In the app, **File → Import Options…** changes header interpretation for CSV, TSV and XLSX, including when the initial open fails. Reopening resets search, filters and sorting. For XLSX, the header is the first occupied row, and the chosen policy also applies when switching worksheets.

**File → Export…** offers the supported formats in the system format menu and compression options for Parquet. The app remembers the last successful export format unless **Remember last export format** is disabled in Settings; the initial default is Parquet with Zstd compression.

## Parquet

Parquet preserves nested types and nulls and is the preferred format for typed round trips. Export uses the selected rows and columns from the captured view.

Export defaults to Zstd compression. The export dialog also offers Snappy, Gzip, Brotli, LZ4 and uncompressed output. Core callers select these through `Export::write_with_options`; plain `write` uses the default. Compression affects storage, not the exported values or types.

## CSV and TSV

Both formats use the first record as column names by default. The app's Import Options and Core's opening options also support headerless input.

Type inference examines the entire input before decoding. Mixed types can become text; mixed integer/fractional columns become text if floating-point promotion would round an integer literal.

Empty fields, including quoted empty strings, read as null. Export uses UTF-8, CRLF line endings and CSV quoting; empty strings and nulls both use empty fields. CSV/TSV do not preserve Arrow schemas and reject nested export columns.

Dates such as `2026-10-30` and ISO-style timestamps such as `2026-10-30T14:30:00.123` can be inferred as temporal types. Locale-specific date strings and standalone times are not automatically recognized as temporal types. Unsupported formats remain text; mixing date values with ordinary text can make the whole column text. There is no user-selectable date parsing pattern. A value that looks like a supported date but is invalid can fail decoding rather than falling back to text.

## XLSX

The default is the first worksheet with its first row used as column names. For workbooks with multiple sheets, select a worksheet in the strip below the table. Switching loads only the requested sheet and resets its search, filters, sorting and analysis selections. If loading fails, the current sheet stays open; the sheet list remains available even if the first sheet cannot be read. Reading uses the occupied worksheet range, trimming blank edges including trailing all-null rows. Formula results come from cached values; Core does not recalculate formulas and rejects cells containing Excel errors.

Export writes one worksheet with a header. Boolean values and ordinary finite numbers use native cells. Integers exceeding 15 digits, decimals, dates/times and non-finite numbers use literal text to preserve displayed values. Nested columns are rejected. This is table export, not workbook, styling or formula round-trip editing.

Date/time cells recognized by the workbook reader become timezone-free, millisecond timestamps; recognized elapsed durations become millisecond durations. Ordinary text cells are not inferred as dates. ISO date cells are handled separately, with timezone-bearing or higher-precision values retained as text rather than losing information. Invalid Excel dates that cannot be converted faithfully also remain text. The original workbook's date display formatting is not retained.

> [!IMPORTANT]
> An XLSX worksheet allows **1,048,576 rows and 16,384 columns**. Gewu reserves a header row when exporting, leaving **1,048,575 data rows**. Date/time exports are text, not native Excel date cells.
