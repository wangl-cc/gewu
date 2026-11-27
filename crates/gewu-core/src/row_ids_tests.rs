use super::*;

#[test]
fn row_ids_and_windows_behave_consistently() {
    let all = RowIds::All(5);
    assert_eq!(all.len(), 5);
    assert!(!all.is_empty());
    assert!(all.ptr_eq(&RowIds::All(5)));
    assert!(!all.ptr_eq(&RowIds::All(4)));

    let slice = all.as_slice();
    assert_eq!(slice.len(), 5);
    assert_eq!(RowSlice::All(0).len(), 0);
    assert_eq!(slice.iter_u32().collect::<Vec<_>>(), [0, 1, 2, 3, 4]);

    let win = slice.window(1, 4);
    assert_eq!(win.len(), 3);
    assert_eq!(win.contiguous_range(), Some(1..4));
    assert_eq!(win.iter().collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(win.iter_u32().collect::<Vec<_>>(), [1, 2, 3]);

    let selected = RowIds::Selected(Arc::from([10_u32, 11, 12, 20]));
    let cloned = selected.clone();
    assert!(selected.ptr_eq(&cloned));
    assert!(!selected.ptr_eq(&all));
    assert!(!selected.ptr_eq(&RowIds::Selected(Arc::from([10_u32, 11, 12, 20]))));

    let sel_slice = selected.as_slice();
    assert_eq!(sel_slice.window(0, 3).contiguous_range(), Some(10..13));
    assert_eq!(sel_slice.window(0, 4).contiguous_range(), None);
    assert_eq!(sel_slice.window(4, 4).contiguous_range(), Some(0..0));
    assert_eq!(sel_slice.window(4, 4).len(), 0);
}
