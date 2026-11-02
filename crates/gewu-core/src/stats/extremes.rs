use std::cmp::Ordering;

use arrow::{array::make_comparator, compute::SortOptions};

use super::ArrayView;
use crate::Result;

/// Rows holding the smallest and largest non-null values among `view`'s rows, in
/// the column's own order. `None` for types Arrow cannot compare.
pub fn extremes(view: ArrayView<'_>) -> Result<(Option<usize>, Option<usize>)> {
    let Ok(cmp) = make_comparator(view.array.as_ref(), view.array.as_ref(), SortOptions::default())
    else {
        return Ok((None, None));
    };
    let (mut lo, mut hi) = (None::<usize>, None::<usize>);
    for r in view.rows.iter().filter(|&r| view.is_valid(r)) {
        lo = Some(match lo {
            None => r,
            Some(prev) => {
                if cmp(r, prev) == Ordering::Less {
                    r
                } else {
                    prev
                }
            }
        });
        hi = Some(match hi {
            None => r,
            Some(prev) => {
                if cmp(r, prev) == Ordering::Greater {
                    r
                } else {
                    prev
                }
            }
        });
    }
    Ok((lo, hi))
}
