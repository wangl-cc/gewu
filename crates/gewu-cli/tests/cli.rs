//! Smoke tests: run the real binary and read its JSON.

use std::{path::PathBuf, process::Command, sync::Arc};

use arrow::array::{ArrayRef, Decimal128Array, Float64Array, Int64Array, RecordBatch, StringArray};
use parquet::arrow::ArrowWriter;
use serde_json::Value;

fn sample(tag: &str) -> PathBuf {
    let labels: ArrayRef = Arc::new(StringArray::from(vec![
        Some("cat"),
        Some("dog"),
        None,
        Some("cat"),
    ]));
    let prices: ArrayRef = Arc::new(Float64Array::from(vec![
        Some(3.5),
        Some(1.0),
        Some(9.0),
        None,
    ]));
    let ids: ArrayRef = Arc::new(Int64Array::from(vec![10, 20, 30, 40]));
    let batch = RecordBatch::try_from_iter(vec![("label", labels), ("price", prices), ("id", ids)])
        .unwrap();
    let path = std::env::temp_dir().join(format!("gewu-cli-{tag}-{}.parquet", std::process::id()));
    let file = std::fs::File::create(&path).unwrap();
    let mut w = ArrowWriter::try_new(file, batch.schema(), None).unwrap();
    w.write(&batch).unwrap();
    w.close().unwrap();
    path
}

fn gewu(args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_gewu"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "gewu {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn ids(v: &Value) -> Vec<String> {
    v["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn schema_lists_columns() {
    let p = sample("schema");
    let v = gewu(&["schema", p.to_str().unwrap()]);
    assert_eq!(v["rows"], 4);
    assert_eq!(v["format"], "Parquet");
    assert_eq!(v["row_groups"], 1);
    assert!(v["compression"].is_array());
    assert_eq!(v["columns"][1]["name"], "price");
    std::fs::remove_file(p).ok();
}

#[test]
fn delimited_files_support_schema_and_queries() {
    for (extension, separator, format) in [("csv", ",", "CSV"), ("tsv", "\t", "TSV")] {
        let path = std::env::temp_dir().join(format!(
            "gewu-cli-delimited-{}.{}",
            std::process::id(),
            extension
        ));
        std::fs::write(
            &path,
            format!("label{separator}id\ncat{separator}10\ndog{separator}20\ncat{separator}30\n"),
        )
        .unwrap();
        let file = path.to_str().unwrap();
        let info = gewu(&["schema", file]);
        assert_eq!(info["format"], format);
        assert_eq!(info["rows"], 3);
        assert!(info.get("row_groups").is_none());
        assert!(info.get("compression").is_none());
        assert_eq!(info["columns"][1]["name"], "id");
        let result = gewu(&["rows", file, "--where", "label=cat", "--sort", "id:desc"]);
        assert_eq!(ids(&result), ["30", "10"]);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn xlsx_sheet_option_and_metadata() {
    use gewu_core::{Document, ExportOptions, FileType};

    let source = sample("xlsx-source");
    let workbook = source.with_extension("xlsx");
    let doc = Document::open(&source).unwrap();
    doc.prepare_export(
        doc.current_view().version,
        &[0, 1, 2],
        FileType::Xlsx,
        ExportOptions::default(),
    )
    .unwrap()
    .write(&workbook)
    .unwrap();
    let file = workbook.to_str().unwrap();
    let info = gewu(&["schema", file]);
    assert_eq!(info["format"], "XLSX");
    assert_eq!(info["rows"], 4);
    let sheet = info["sheet"].as_str().unwrap();
    assert_eq!(info["sheets"], serde_json::json!([sheet]));
    assert!(info.get("row_groups").is_none());
    // The global option works on either side of the subcommand.
    let rows = gewu(&["--sheet", sheet, "rows", file, "--sort", "id:desc"]);
    assert_eq!(ids(&rows), ["40.0", "30.0", "20.0", "10.0"]);
    assert_eq!(gewu(&["schema", file, "--sheet", sheet]), info);

    let out = Command::new(env!("CARGO_BIN_EXE_gewu"))
        .args(["rows", file, "--sheet", "missing-sheet"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let error: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert!(error["error"].as_str().unwrap().contains("missing-sheet"));
    std::fs::remove_file(source).unwrap();
    std::fs::remove_file(workbook).unwrap();
}

#[test]
fn rows_with_filters_and_sort() {
    let p = sample("rows");
    let f = p.to_str().unwrap();
    let v = gewu(&["rows", f, "--where", "label=cat", "--sort", "id:desc"]);
    assert_eq!(v["matched_rows"], 2);
    assert_eq!(ids(&v), ["40", "10"]);

    let v = gewu(&["rows", f, "--range", "price=2..", "--sort", "id"]);
    assert_eq!(ids(&v), ["10", "30"]);

    // One value of a number column, and bounds read in the column's type.
    let v = gewu(&["rows", f, "--where", "id=30"]);
    assert_eq!(ids(&v), ["30"]);
    let v = gewu(&["rows", f, "--range", "id=15..30.5", "--sort", "id"]);
    assert_eq!(ids(&v), ["20", "30"]);

    let v = gewu(&["rows", f, "--null", "label"]);
    assert_eq!(ids(&v), ["30"]);

    let v = gewu(&["rows", f, "--search", "dog"]);
    assert_eq!(ids(&v), ["20"]);
    std::fs::remove_file(p).ok();
}

#[test]
fn decimal_filters_preserve_text_beyond_float_precision() {
    let prices: ArrayRef = Arc::new(
        Decimal128Array::from(vec![9_007_199_254_740_992_i128, 9_007_199_254_740_993])
            .with_precision_and_scale(20, 2)
            .unwrap(),
    );
    let identifiers: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
    let batch = RecordBatch::try_from_iter(vec![("price", prices), ("id", identifiers)]).unwrap();
    let path =
        std::env::temp_dir().join(format!("gewu-cli-decimal-{}.parquet", std::process::id()));
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = ArrowWriter::try_new(file, batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    let file = path.to_str().unwrap();

    assert_eq!(
        ids(&gewu(&["rows", file, "--where", "price=90071992547409.93"])),
        ["2"]
    );
    assert_eq!(
        ids(&gewu(&[
            "rows",
            file,
            "--range",
            "price=90071992547409.925.."
        ])),
        ["2"]
    );
    assert_eq!(
        ids(&gewu(&[
            "rows",
            file,
            "--range",
            "price=..90071992547409.925"
        ])),
        ["1"]
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn distinct_hist_and_stats() {
    let p = sample("facets");
    let f = p.to_str().unwrap();
    let v = gewu(&["distinct", f, "label"]);
    assert_eq!(v["total"], 3);

    let v = gewu(&["hist", f, "id", "--bins", "4"]);
    assert_eq!(v["rows"], 4);
    let counts: u64 = v["bins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["count"].as_u64().unwrap())
        .sum();
    assert_eq!(counts, 4);

    let v = gewu(&["stats", f, "price"]);
    assert_eq!(v["null_count"], 1);
    assert_eq!(v["min"], "1.0");
    assert_eq!(v["max"], "9.0");

    // Stats follow the view; text columns list their most common values.
    let v = gewu(&["stats", f, "label", "--range", "id=..30"]);
    assert_eq!(v["rows"], 3);
    assert_eq!(v["null_count"], 1);
    assert_eq!(v["top_values"][0]["value"], "cat");
    std::fs::remove_file(p).ok();
}

#[test]
fn errors_are_json_on_stderr() {
    let p = sample("errors");
    let out = Command::new(env!("CARGO_BIN_EXE_gewu"))
        .args(["rows", p.to_str().unwrap(), "--sort", "nope"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert!(err["error"].as_str().unwrap().contains("nope"));
    std::fs::remove_file(p).ok();
}
