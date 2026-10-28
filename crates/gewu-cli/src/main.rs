//! `gewu` command line. Every command prints JSON so scripts and agents can
//! read the output.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use gewu_core::{Document, Filter, SortKey, ViewState};
use serde_json::{json, Value};

#[derive(Parser)]
#[command(name = "gewu", version, about = "Look inside Parquet files")]
struct Cli {
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
    /// Null count, distinct count, min and max of a column.
    Stats { file: PathBuf, column: String },
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
    let out = match cli.command {
        Command::Schema { file } => schema(&open(&file)?),
        Command::Rows {
            file,
            view,
            columns,
            offset,
            limit,
        } => rows(&open(&file)?, &view, &columns, offset, limit)?,
        Command::Distinct {
            file,
            column,
            view,
            limit,
        } => {
            let doc = open(&file)?;
            let col = column_index(&doc, &column)?;
            let state = view_state(&doc, &view)?;
            let d = doc.distinct_values(col, Some(&state), limit)?;
            json!({
                "column": column,
                "total": d.total,
                "truncated": d.truncated,
                "values": d.values.iter().map(|v| json!({
                    "value": v.label,
                    "count": v.count,
                })).collect::<Vec<_>>(),
            })
        }
        Command::Stats { file, column } => {
            let doc = open(&file)?;
            let col = column_index(&doc, &column)?;
            let s = doc.column_stats(col)?;
            json!({
                "column": column,
                "type": doc.info().columns[col as usize].type_name,
                "rows": doc.info().row_count,
                "null_count": s.null_count,
                "distinct_count": s.distinct_count,
                "min": s.min,
                "max": s.max,
            })
        }
    };
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

fn open(file: &PathBuf) -> Result<Document> {
    Document::open(file).with_context(|| format!("cannot open {}", file.display()))
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
        let keys = doc.keys_for_labels(column, &labels)?;
        state.filters.push(Filter::InValues { column, keys });
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
    Ok(state)
}

fn schema(doc: &Document) -> Value {
    let info = doc.info();
    json!({
        "path": info.path,
        "rows": info.row_count,
        "row_groups": info.row_groups,
        "file_size": info.file_size,
        "compression": info.compression,
        "columns": info.columns.iter().map(|c| json!({
            "name": c.name,
            "type": c.type_name,
            "nullable": c.nullable,
        })).collect::<Vec<_>>(),
    })
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
