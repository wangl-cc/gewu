use crate::GewuDataset;

fn session(csv: &str) -> (std::path::PathBuf, std::sync::Arc<super::GewuFind>) {
    let source = std::env::temp_dir().join(format!(
        "gewu-uniffi-find-{}-{}.csv",
        std::process::id(),
        csv.len()
    ));
    std::fs::write(&source, csv).unwrap();
    let dataset =
        GewuDataset::open(source.to_str().unwrap().to_owned(), Default::default()).unwrap();
    let columns = dataset
        .info()
        .columns
        .iter()
        .map(|column| column.id)
        .collect();
    (source, dataset.rows().find("12".into(), columns).unwrap())
}

#[test]
fn steps_wrap_and_reduce_whole_passes() {
    let (source, find) = session("value,other\nhit,12\nmiss,120\n12,x\n");
    let cell = |row, column| {
        Some(super::CellMatch {
            display_row: row,
            column: gewu_core::ColumnId::from_raw(column),
        })
    };
    let all = [cell(0, 1), cell(1, 1), cell(2, 0)];
    assert_eq!(find.step(None, 1).unwrap(), all[0]);
    assert_eq!(find.step(None, -1).unwrap(), all[2]);
    assert_eq!(find.step(all[0], 2).unwrap(), all[2]);
    assert_eq!(find.step(all[2], 1).unwrap(), all[0], "wraps forward");
    assert_eq!(find.step(all[0], -1).unwrap(), all[2], "wraps backward");
    assert_eq!(find.step(all[1], 3 * 1000 + 1).unwrap(), all[2]);
    assert_eq!(find.step(all[1], -(3 * 1000 + 2)).unwrap(), all[2]);
    assert_eq!(find.step(None, 0).unwrap(), None);
    std::fs::remove_file(source).unwrap();

    let (source, empty) = session("value\nnone\n");
    assert_eq!(empty.step(None, 5).unwrap(), None);
    std::fs::remove_file(source).unwrap();
}

#[test]
fn highlights_span_more_rows_than_one_core_block() {
    let mut csv = String::from("value\n");
    for row in 0..600 {
        csv.push_str(if row % 300 == 7 { "12\n" } else { "0\n" });
    }
    let (source, find) = session(&csv);
    let column = gewu_core::ColumnId::from_raw(0);
    let highlight = find.highlight(vec![column], 0, 1000).unwrap();
    assert_eq!((highlight.start, highlight.n_rows), (0, 600));
    let bits = &highlight.columns[0].matched;
    assert_eq!(bits.len(), 75);
    let hits: Vec<_> = (0..600)
        .filter(|row| bits[row / 8] & (1 << (row % 8)) != 0)
        .collect();
    assert_eq!(hits, [7, 307]);
    std::fs::remove_file(source).unwrap();
}
