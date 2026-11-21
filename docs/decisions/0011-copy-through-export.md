# 0011. Copy goes through the export path

- Status: accepted (2026-11-20, second round of #17)
- Evidence: [frontend redesign](https://claude.ai/artifact/VRWoMrsXKZJRebQywfifcy) ("第二轮"); `OrderedView::export_rows`, `Export::write_to_memory`; GewuKit `Table/DelimitedText.swift`; App `Table/CopyModel.swift`.

## Context

Copy used to read display text column by column and assemble TSV in Swift. That made a second serializer with its own quoting, line endings and value formatting, which could drift from TSV export.

## Decision

- Copy is a TSV export of the selected display rows and visible columns, written into memory: `export_rows(ranges, columns, Tsv)` then `write_to_memory(options, limit)`.
- Nested values are written as display text. This is an export option (`ExportOptions::nested_as_text`), on for copy; Core callers can choose it for file export, and the App turns it on for file export too.
- The in-memory write has a byte cap. Beyond it, export fails with a `SizeLimit` kind and the App suggests File > Export….
- Swift reads the result in place through the data plane ([ADR 0008](0008-data-plane.md)) and copies it once into the pasteboard.

## Rejected alternatives

- **Building TSV in Swift from displayed text.** A second serializer.
- **A dedicated copy API in Core.** Would duplicate export's row selection, encoding and limits.

## Consequences

- "What does copy produce?" has one answer: exactly a TSV export of those rows. [File formats](../file-formats.md#csv-and-tsv) documents it.
- Changes to TSV export change copy. Test the shared behavior at the export boundary.
