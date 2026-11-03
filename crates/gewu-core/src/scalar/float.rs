//! Floating-point projections of exact integer endpoints.

pub(crate) fn integer_endpoint(text: &str, rounded: f64, lower: bool) -> f64 {
    let integer = text
        .parse::<i64>()
        .map(i128::from)
        .or_else(|_| text.parse::<u64>().map(i128::from));
    let Ok(integer) = integer else {
        return rounded;
    };
    integer_value_endpoint(integer, rounded, lower)
}

pub(crate) fn integer_value_endpoint(integer: i128, rounded: f64, lower: bool) -> f64 {
    if lower && (rounded as i128) < integer {
        rounded.next_up()
    } else if !lower && (rounded as i128) > integer {
        rounded.next_down()
    } else {
        rounded
    }
}
