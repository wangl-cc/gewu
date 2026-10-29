//! `gewu` command line. Every command prints JSON so scripts and agents can
//! read the output.

use std::{collections::BTreeMap, path::PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use gewu_core::{Document, FileMetadata, Filter, OpenOptions, SortKey, ViewState};
use serde_json::{json, Value};

#[derive(Parser)]
#[command(
    name = "gewu",
    version,
    about = "Look inside Parquet, CSV, TSV and XLSX files"
)]
struct Cli {
    /// XLSX worksheet to read (default: the first worksheet).
    #[arg(long, global = true)]
    sheet: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Columns, row count and file metadata.
    Schema { file: PathBuf },
    /// Rows after sorting and filtering.
    Rows {
        file: PathBuf,
        #[command(flatten)]
        view: ViewArgs,
        /// Columns to print, comma separated (default: all).
        #[arg(long, value_delimiter = ',')]
        columns: Vec<String>,
        #[arg(long, default_value_t = 0)]
        offset: u64,
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Distinct values of a column with their counts.
    Distinct {
        file: PathBuf,
        column: String,
        #[command(flatten)]
        view: ViewArgs,
        #[arg(long, default_value_t = 100)]
        limit: u32,
    },
    /// Null count, distinct count, min, max and more for a column, over the
    /// rows the view keeps.
    Stats {
        file: PathBuf,
        column: String,
        #[command(flatten)]
        view: ViewArgs,
        /// Most common values to list (text and other non-numeric columns).
        #[arg(long, default_value_t = 10)]
        top: u32,
    },
    /// Distribution of a numeric, date or time column.
    Hist {
        file: PathBuf,
        column: String,
        #[command(flatten)]
        view: ViewArgs,
        #[arg(long, default_value_t = 20)]
        bins: u32,
    },
}

#[derive(Args, Default)]
struct ViewArgs {
    /// Sort by a column, e.g. `price` or `price:desc`. Repeat for more keys.
    #[arg(long)]
    sort: Vec<String>,
    /// Keep rows where column equals value, e.g. `label=cat`. Repeating the
    /// same column means "any of these values".
    #[arg(long = "where")]
    where_eq: Vec<String>,
    /// Keep rows where the column is null.
    #[arg(long)]
    null: Vec<String>,
    /// Keep rows where the column is not null.
    #[arg(long)]
    not_null: Vec<String>,
    /// Keep rows where the column contains text, e.g. `name=smith`.
    #[arg(long)]
    contains: Vec<String>,
    /// Keep rows whose value is in a range, e.g. `price=10..99.5`,
    /// `id=1000..`, `price=..5` or `day=2024-01-01..2024-03-31`. Both ends
    /// are inclusive.
    #[arg(long)]
    range: Vec<String>,
    /// Keep rows where any column contains this text.
    #[arg(long)]
    search: Option<String>,
}

fn main() {
    if let Err(e) = run(Cli::parse()) {
        let err = json!({ "error": format!("{e:#}") });
        eprintln!("{}", serde_json::to_string_pretty(&err).unwrap());
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let options = OpenOptions {
        sheet: cli.sheet,
        ..Default::default()
    };
    let out = match cli.command {
        Command::Schema { file } => schema(&open(&file, &options)?),
        Command::Rows {
            file,
            view,
            columns,
            offset,
            limit,
        } => rows(&open(&file, &options)?, &view, &columns, offset, limit)?,
        Command::Distinct {
            file,
            column,
            view,
            limit,
        } => {
            let doc = open(&file, &options)?;
            let col = column_index(&doc, &column)?;
            let state = view_state(&doc, &view)?;
            let d = doc.distinct_values(col, Some(&state), limit)?;
            json!({
                "column": column,
                "total": d.total,
                "truncated": d.truncated,
                // Too many values to list them all: these are the most common.
                "most_common_only": !d.keyed,
                "values": d.values.iter().map(|v| json!({
                    "value": v.label,
                    "count": v.count,
                })).collect::<Vec<_>>(),
            })
        }
        Command::Hist {
            file,
            column,
            view,
            bins,
        } => {
            let doc = open(&file, &options)?;
            let col = column_index(&doc, &column)?;
            let state = view_state(&doc, &view)?;
            let h = doc.histogram(col, Some(&state), bins)?;
            let width = match (h.lo, h.hi) {
                (Some(lo), Some(hi)) if !h.counts.is_empty() => (hi - lo) / h.counts.len() as f64,
                _ => 0.0,
            };
            json!({
                "column": column,
                "rows": h.total,
                "null_count": h.null_count,
                "nan_count": h.nan_count,
                "bins": h.counts.iter().enumerate().map(|(i, n)| {
                    let lo = h.lo.unwrap_or(0.0) + width * i as f64;
                    let mut bin = json!({ "from": lo, "to": lo + width, "count": n });
                    // Integer columns include dates and times: also show the
                    // bin start the way the table displays it.
                    if h.integer {
                        if let Ok(text) = doc.format_value(col, lo.ceil() as i64) {
                            bin["from_text"] = json!(text);
                        }
                    }
                    bin
                }).collect::<Vec<_>>(),
            })
        }
        Command::Stats {
            file,
            column,
            view,
            top,
        } => {
            let doc = open(&file, &options)?;
            let col = column_index(&doc, &column)?;
            doc.apply_view(&view_state(&doc, &view)?)?;
            let s = doc.column_summary(col, top)?;
            let mut out = json!({
                "column": column,
                "type": doc.info().columns[col as usize].type_name,
                "rows": s.rows,
                "null_count": s.null_count,
                "distinct_count": s.distinct_count,
                "min": s.min_label,
                "max": s.max_label,
            });
            if doc.column_capabilities(col)?.histogram {
                out["nan_count"] = json!(s.nan_count);
                out["mean"] = json!(s.mean);
                out["std_dev"] = json!(s.std_dev);
            } else {
                out["top_values"] = s
                    .top_values
                    .iter()
                    .map(|v| json!({ "value": v.label, "count": v.count }))
                    .collect();
            }
            out
        }
    };
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

fn open(file: &PathBuf, options: &OpenOptions) -> Result<Document> {
    Document::open_with_options(file, options)
        .with_context(|| format!("cannot open {}", file.display()))
}

fn column_index(doc: &Document, name: &str) -> Result<u32> {
    doc.column_index(name)
        .ok_or_else(|| anyhow!("no column named {name:?}"))
}

fn split_pair<'a>(arg: &'a str, flag: &str) -> Result<(&'a str, &'a str)> {
    arg.split_once('=')
        .ok_or_else(|| anyhow!("--{flag} expects column=value, got {arg:?}"))
}

fn view_state(doc: &Document, args: &ViewArgs) -> Result<ViewState> {
    let mut state = ViewState {
        search: args.search.clone(),
        ..Default::default()
    };

    for s in &args.sort {
        let (name, descending) = match s.rsplit_once(':') {
            Some((n, "desc")) => (n, true),
            Some((n, "asc")) => (n, false),
            _ => (s.as_str(), false),
        };
        state.sort.push(SortKey {
            column: column_index(doc, name)?,
            descending,
        });
    }

    let mut eq: BTreeMap<u32, Vec<Option<String>>> = BTreeMap::new();
    for w in &args.where_eq {
        let (name, value) = split_pair(w, "where")?;
        eq.entry(column_index(doc, name)?)
            .or_default()
            .push(Some(value.to_string()));
    }
    for (column, labels) in eq {
        let ordered = doc.column_capabilities(column)?.range;
        if let (true, [Some(value)]) = (ordered, labels.as_slice()) {
            // One value of a number, date or time: no value index needed.
            state.filters.push(Filter::Equals {
                column,
                value: value.clone(),
            });
        } else {
            let keys = doc.keys_for_labels(column, &labels)?;
            state.filters.push(Filter::InValues { column, keys });
        }
    }

    for name in &args.null {
        state.filters.push(Filter::IsNull {
            column: column_index(doc, name)?,
            negate: false,
        });
    }
    for name in &args.not_null {
        state.filters.push(Filter::IsNull {
            column: column_index(doc, name)?,
            negate: true,
        });
    }
    for c in &args.contains {
        let (name, text) = split_pair(c, "contains")?;
        state.filters.push(Filter::Contains {
            column: column_index(doc, name)?,
            text: text.to_string(),
            case_sensitive: false,
        });
    }
    for r in &args.range {
        let (name, span) = split_pair(r, "range")?;
        let (lo, hi) = span
            .split_once("..")
            .ok_or_else(|| anyhow!("--range expects column=lo..hi, got {r:?}"))?;
        let column = column_index(doc, name)?;
        state.filters.push(Filter::Range {
            column,
            min: bound_text(lo)
                .map(|text| doc.parse_bound(column, text))
                .transpose()?,
            max: bound_text(hi)
                .map(|text| doc.parse_bound(column, text))
                .transpose()?,
        });
    }
    Ok(state)
}

/// Empty is an open end; preserve other inputs for the column's parser.
fn bound_text(s: &str) -> Option<&str> {
    let s = s.trim();
    (!s.is_empty()).then_some(s)
}

fn schema(doc: &Document) -> Value {
    let info = doc.info();
    let mut out = json!({
        "path": info.path,
        "format": info.format.to_string(),
        "rows": info.row_count,
        "file_size": info.file_size,
        "columns": info.columns.iter().map(|c| json!({
            "name": c.name,
            "type": c.type_name,
            "nullable": c.nullable,
        })).collect::<Vec<_>>(),
    });
    match &info.metadata {
        FileMetadata::Parquet {
            row_groups,
            compression,
            ..
        } => {
            out["row_groups"] = json!(row_groups);
            out["compression"] = json!(compression);
        }
        FileMetadata::Delimited => {}
        FileMetadata::Xlsx { sheet, sheets } => {
            out["sheet"] = json!(sheet);
            out["sheets"] = json!(sheets);
        }
    }
    out
}

fn rows(
    doc: &Document,
    view: &ViewArgs,
    columns: &[String],
    offset: u64,
    limit: u32,
) -> Result<Value> {
    let state = view_state(doc, view)?;
    let v = doc.apply_view(&state)?;

    let cols: Vec<u32> = if columns.is_empty() {
        (0..doc.info().columns.len() as u32).collect()
    } else {
        columns
            .iter()
            .map(|c| column_index(doc, c))
            .collect::<Result<_>>()?
    };
    if cols.is_empty() {
        bail!("no columns to print");
    }
    let names: Vec<&str> = cols
        .iter()
        .map(|&c| doc.info().columns[c as usize].name.as_str())
        .collect();

    let block = doc.rows(v.version, offset, limit, &cols)?;
    let rows: Vec<Value> = (0..block.n_rows)
        .map(|r| {
            let mut obj = serde_json::Map::new();
            for (c, name) in names.iter().enumerate() {
                obj.insert(name.to_string(), json!(block.cell(r, c as u32)));
            }
            Value::Object(obj)
        })
        .collect();

    Ok(json!({
        "total_rows": doc.info().row_count,
        "matched_rows": v.row_count,
        "offset": block.start,
        "returned_rows": block.n_rows,
        "rows": rows,
    }))
}
