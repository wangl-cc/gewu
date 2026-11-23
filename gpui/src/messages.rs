/// Format counts whose English noun takes a regular plural.
pub(crate) fn counted(n: u64, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

pub(crate) fn omitted_pairs(count: u64) -> Option<String> {
    (count > 0).then(|| format!("{} omitted", counted(count, "nonpositive pair")))
}

pub(crate) fn omitted_values(nonpositive: u64, infinite: u64) -> Option<String> {
    let count = match (nonpositive, infinite) {
        (0, 0) => return None,
        (count, 0) => counted(count, "nonpositive value"),
        (0, count) => counted(count, "infinite value"),
        (nonpositive, infinite) => format!("{nonpositive} nonpositive, {infinite} infinite values"),
    };
    Some(format!("{count} omitted"))
}
