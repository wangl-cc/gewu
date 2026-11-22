use gewu_core::{CellMatch, Find, FindError, MAX_FIND_PAGE_SIZE};

/// Navigate a signed number of matching cells, paging within Core's limit and wrapping
/// at either end. The flag records whether navigation passed the end of the match list.
pub fn step(
    find: &Find,
    from: Option<CellMatch>,
    by: i64,
) -> Result<(Option<CellMatch>, bool), FindError> {
    if by == 0 {
        return Ok((from, false));
    }
    let forward = by > 0;
    let mut wrapped = false;
    let mut remaining = by.unsigned_abs();
    let mut cursor = from;
    // Matches passed since the last start from outside the cells; once a
    // whole pass is known, the remaining steps reduce modulo its length.
    let mut passed: Option<u64> = from.is_none().then_some(0);
    loop {
        let limit = remaining.min(u64::from(MAX_FIND_PAGE_SIZE)) as u32;
        let mut page = if forward {
            find.next(cursor.as_ref(), limit)?
        } else {
            find.previous(cursor.as_ref(), limit)?
        };
        if page.is_empty() {
            match passed {
                Some(0) => return Ok((None, false)),
                Some(total) => remaining = (remaining - 1) % total + 1,
                None => {}
            }
            wrapped = true;
            cursor = None;
            passed = Some(0);
            continue;
        }
        if !forward {
            page.reverse();
        }
        let taken = page.len() as u64;
        if remaining <= taken {
            return Ok((Some(page[remaining as usize - 1]), wrapped));
        }
        remaining -= taken;
        cursor = page.last().copied();
        if let Some(passed) = &mut passed {
            *passed += taken;
        }
    }
}

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
