/// Format a display count with the English UI's digit grouping.
pub(crate) fn format_count(n: u64) -> String {
    let mut text = n.to_string();
    let mut index = text.len();
    while index > 3 {
        index -= 3;
        text.insert(index, ',');
    }
    text
}

/// Group the whole-number part of an integer or decimal statistic text with commas.
pub(crate) fn format_grouped_number(text: &str) -> String {
    let (sign, digits) = text
        .strip_prefix('-')
        .map_or(("", text), |rest| ("-", rest));
    let (whole, fraction) = digits
        .find('.')
        .map_or((digits, ""), |dot| digits.split_at(dot));
    if whole.len() > 3
        && whole.bytes().all(|b| b.is_ascii_digit())
        && (fraction.is_empty()
            || (fraction.len() > 1 && fraction[1..].bytes().all(|b| b.is_ascii_digit())))
    {
        let mut formatted = String::with_capacity(text.len() + whole.len() / 3);
        formatted.push_str(sign);
        for (offset, byte) in whole.bytes().enumerate() {
            if offset > 0 && (whole.len() - offset).is_multiple_of(3) {
                formatted.push(',');
            }
            formatted.push(byte as char);
        }
        formatted.push_str(fraction);
        return formatted;
    }
    text.to_owned()
}

/// Format a floating-point value to at most `digits` significant figures.
pub(crate) fn significant_digits(value: f64, digits: usize) -> String {
    if !value.is_finite() || value == 0. {
        return value.to_string();
    }
    let exp = value.abs().log10().floor() as i32;
    let digits = digits.clamp(1, 17);
    if exp < -4 || exp >= digits as i32 {
        let text = format!("{:.*e}", digits - 1, value);
        if let Some((coefficient, power)) = text.split_once('e') {
            let short = if coefficient.contains('.') {
                coefficient.trim_end_matches('0').trim_end_matches('.')
            } else {
                coefficient
            };
            return format!("{short}e{power}");
        }
        return text;
    }
    let decimals = (digits as i32 - exp - 1) as usize;
    let text = format!("{value:.decimals$}");
    if decimals == 0 {
        text
    } else {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

/// Format a floating-point statistic with thousands separators across human-readable magnitudes.
pub(crate) fn format_numeric_statistic(value: f64, digits: usize) -> String {
    if !value.is_finite() || value == 0. {
        return value.to_string();
    }
    let exp = value.abs().log10().floor() as i32;
    let digits = digits.clamp(1, 17);
    if (-4..9).contains(&exp) {
        if exp >= digits as i32 - 1 {
            let scale = 10f64.powi(exp - digits as i32 + 1);
            let rounded = (value / scale).round() * scale;
            return format_grouped_number(&format!("{rounded:.0}"));
        }
        let decimals = (digits as i32 - exp - 1) as usize;
        let text = format!("{value:.decimals$}");
        let trimmed = if decimals == 0 {
            text.as_str()
        } else {
            text.trim_end_matches('0').trim_end_matches('.')
        };
        return format_grouped_number(trimmed);
    }
    significant_digits(value, digits)
}

/// Format counts whose English noun takes a regular plural.
pub(crate) fn counted(n: u64, noun: &str) -> String {
    format!(
        "{} {noun}{}",
        format_count(n),
        if n == 1 { "" } else { "s" }
    )
}

pub(crate) fn omitted_pairs(count: u64) -> Option<String> {
    (count > 0).then(|| format!("{} omitted", counted(count, "nonpositive pair")))
}

pub(crate) fn omitted_values(nonpositive: u64, infinite: u64) -> Option<String> {
    let count = match (nonpositive, infinite) {
        (0, 0) => return None,
        (count, 0) => counted(count, "nonpositive value"),
        (0, count) => counted(count, "infinite value"),
        (nonpositive, infinite) => format!(
            "{} nonpositive, {} infinite values",
            format_count(nonpositive),
            format_count(infinite),
        ),
    };
    Some(format!("{count} omitted"))
}

#[cfg(test)]
#[path = "messages_tests.rs"]
mod tests;
