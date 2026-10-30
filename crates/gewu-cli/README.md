# gewu CLI

The `gewu` CLI provides JSON output for scripts and AI agents. It also supports selecting an XLSX worksheet by name.

Run from the repository root:

```sh
cargo install --path crates/gewu-cli
gewu schema data.parquet
gewu rows data.csv --search example --limit 20
gewu rows workbook.xlsx --sheet Data --limit 20
gewu rows data.parquet --sort price:desc --range price=10..99.5 --limit 20
gewu stats data.parquet price --where label=cat
gewu --help
```

See [search and filtering](../../docs/search-and-filter.md) for condition composition and [data types](../../docs/data-types.md) for typed matching rules.
