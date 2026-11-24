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
