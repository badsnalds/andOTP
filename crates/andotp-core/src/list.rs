//! Sorting, searching and tag filtering of entries.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::Entry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortMode {
    #[default]
    Unsorted,
    Issuer,
    Label,
    LastUsed,
    MostUsed,
}

impl SortMode {
    pub const ALL: [SortMode; 5] = [Self::Unsorted, Self::Issuer, Self::Label, Self::LastUsed, Self::MostUsed];

    pub fn label(self) -> &'static str {
        match self {
            Self::Unsorted => "Manual",
            Self::Issuer => "Issuer",
            Self::Label => "Label",
            Self::LastUsed => "Last used",
            Self::MostUsed => "Most used",
        }
    }
}

/// How multiple selected tags are combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagMode {
    /// Entries carrying any of the selected tags.
    #[default]
    Or,
    /// Entries carrying all selected tags.
    And,
}

/// Case-insensitive comparison, roughly equivalent to a primary-strength collator.
fn text_cmp(a: &str, b: &str) -> Ordering {
    a.to_lowercase().cmp(&b.to_lowercase())
}

fn compare(mode: SortMode, a: &Entry, b: &Entry) -> Ordering {
    match mode {
        SortMode::Unsorted => Ordering::Equal,
        SortMode::Issuer => text_cmp(&a.issuer, &b.issuer).then_with(|| text_cmp(&a.label, &b.label)),
        SortMode::Label => text_cmp(&a.label, &b.label).then_with(|| text_cmp(&a.issuer, &b.issuer)),
        SortMode::LastUsed => b.last_used.cmp(&a.last_used),
        SortMode::MostUsed => b.used_frequency.cmp(&a.used_frequency),
    }
}

/// Whether `entry` matches a free-text query in its label, issuer or tags.
pub fn matches_query(entry: &Entry, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || entry.label.to_lowercase().contains(&query)
        || entry.issuer.to_lowercase().contains(&query)
        || entry.tags.iter().any(|t| t.to_lowercase().contains(&query))
}

/// Whether `entry` passes the tag filter. An empty selection matches everything.
pub fn matches_tags(entry: &Entry, tags: &[String], mode: TagMode) -> bool {
    if tags.is_empty() {
        return true;
    }
    match mode {
        TagMode::Or => tags.iter().any(|t| entry.tags.contains(t)),
        TagMode::And => tags.iter().all(|t| entry.tags.contains(t)),
    }
}

/// Returns the indices of the entries that pass the filters, in display order.
pub fn view(entries: &[Entry], query: &str, tags: &[String], tag_mode: TagMode, sort: SortMode) -> Vec<usize> {
    let mut indices: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| matches_query(e, query) && matches_tags(e, tags, tag_mode))
        .map(|(i, _)| i)
        .collect();
    indices.sort_by(|&a, &b| compare(sort, &entries[a], &entries[b]));
    indices
}

/// All tags used by any entry, sorted.
pub fn all_tags(entries: &[Entry]) -> Vec<String> {
    entries.iter().flat_map(|e| e.tags.iter().cloned()).collect::<BTreeSet<_>>().into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OtpType;

    fn entry(issuer: &str, label: &str, tags: &[&str], used: i64) -> Entry {
        let mut e = Entry::new(OtpType::Totp, "MZXW6", issuer, label).unwrap();
        e.tags = tags.iter().map(|t| t.to_string()).collect();
        e.used_frequency = used;
        e
    }

    #[test]
    fn filtering_and_sorting() {
        let entries = vec![
            entry("zeta", "b", &["work"], 1),
            entry("Alpha", "a", &["home", "work"], 5),
            entry("beta", "c", &[], 3),
        ];
        assert_eq!(view(&entries, "", &[], TagMode::Or, SortMode::Unsorted), [0, 1, 2]);
        assert_eq!(view(&entries, "", &[], TagMode::Or, SortMode::Issuer), [1, 2, 0]);
        assert_eq!(view(&entries, "", &[], TagMode::Or, SortMode::MostUsed), [1, 2, 0]);
        assert_eq!(view(&entries, "WORK", &[], TagMode::Or, SortMode::Label), [1, 0]);
        let tags = vec!["home".to_string(), "work".to_string()];
        assert_eq!(view(&entries, "", &tags, TagMode::Or, SortMode::Unsorted), [0, 1]);
        assert_eq!(view(&entries, "", &tags, TagMode::And, SortMode::Unsorted), [1]);
        assert_eq!(all_tags(&entries), ["home", "work"]);
    }
}
