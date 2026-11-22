use std::{io::Write, sync::Arc};

use gewu_core::Dataset;

use super::step;

#[test]
fn navigation_crosses_page_boundaries_wraps_and_cancels_opposite_steps() {
    let mut file = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
    writeln!(file, "name").unwrap();
    for _ in 0..300 {
        writeln!(file, "hit").unwrap();
    }
    let source = Arc::new(Dataset::open(file.path()).unwrap());
    let col = source.column_id("name").unwrap();
    let view = source.unfiltered().unsorted();
    let find = view.find("hit", &[col]).unwrap();
    let (last, wrapped) = step(&find, None, 300).unwrap();
    assert_eq!(last.unwrap().display_row, 299);
    assert!(!wrapped);
    let (first, wrapped) = step(&find, last, 601).unwrap();
    assert_eq!(first.unwrap().display_row, 0);
    assert!(wrapped);
    let (back, wrapped) = step(&find, first, -301).unwrap();
    assert_eq!(back, last);
    assert!(wrapped);
    assert_eq!(step(&find, back, 0).unwrap(), (back, false));
    assert_eq!(
        step(&view.find("absent", &[col]).unwrap(), None, i64::MIN).unwrap(),
        (None, false)
    );
}
