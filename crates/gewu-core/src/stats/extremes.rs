use std::cmp::Ordering;

use arrow::{array::make_comparator, compute::SortOptions};

use super::ArrayView;
use crate::Result;

pub(crate) fn extremes(
    view: ArrayView<'_>,
    cancel: &crate::Cancellation,
) -> Result<(Option<usize>, Option<usize>)> {
    cancel.check()?;
    let Ok(cmp) = make_comparator(
        view.array.as_ref(),
        view.array.as_ref(),
        SortOptions::default(),
    ) else {
        return Ok((None, None));
    };
    let (mut lo, mut hi) = (None::<usize>, None::<usize>);
    for (index, r) in view.rows.iter().enumerate() {
        cancel.check_at(index)?;
        if !view.is_valid(r) {
            continue;
        }
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
