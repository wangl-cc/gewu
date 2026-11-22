use super::*;

#[test]
fn extend_and_collapse() {
    let s = Selection::single(Cell { row: 5, column: 2 });
    let s = s.moved(3, -1, true, 100, 10);
    assert_eq!(s.rows(), 5..9);
    assert_eq!(s.columns(), 1..3);
    let s = s.moved(-100, 100, false, 100, 10);
    assert_eq!(s, Selection::single(Cell { row: 0, column: 9 }));
    let s = s.moved(i64::MAX, 0, true, 50_000_000, 10);
    assert_eq!(s.head.row, 49_999_999);
}
