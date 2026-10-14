//! The analyzer and the BM25F scoring function.
//!
//! One analyzer runs over both documents and queries, so a term can only ever
//! match a whole token — `relay` never matches inside `relayed`. Tokens keep
//! dots and hyphens (`relay.get_target_state` stays one token) because wiki
//! pages name APIs and tickets that way.

use std::collections::{HashMap, HashSet};

use crate::config::{self, BM25_K1, MIN_TERM_LENGTH};

/// Floor for the length-normalization divisor, so a pathological ratio cannot
/// divide by zero.
const MIN_LENGTH_NORM_DIVISOR: f64 = 1e-6;

/// One zone of a document, with its boost and length normalization.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field {
    /// Multiplier applied to this field's term frequencies.
    pub weight: f64,
    /// How strongly the field's length is normalized; zero disables it.
    pub length_norm: f64,
}

/// Per-term token counts for one field of one document.
#[derive(Debug, Clone, Default)]
pub struct FieldCounts {
    /// How many times each term occurs in this field.
    pub counts: HashMap<String, usize>,
    /// Total tokens in this field.
    pub length: usize,
}

/// Corpus-wide statistics shared by every scoring call.
#[derive(Debug, Clone, Default)]
pub struct CorpusStats {
    /// Number of documents in the corpus.
    pub document_count: usize,
    /// Mean token count per field, aligned to the configured fields.
    pub average_lengths: Vec<f64>,
    /// How many documents contain each term, at most once per document.
    pub document_frequency: HashMap<String, usize>,
}

impl CorpusStats {
    /// An empty statistics block, for a corpus with no documents.
    #[cfg(test)]
    pub fn empty() -> Self {
        Self::default()
    }
}

/// Splits text into lowercase searchable terms.
///
/// A term is a run starting with `[a-z0-9]` and continuing with `[a-z0-9_.-]*`.
/// The continuation set is deliberately unconditional: `release.` and
/// `self._private` are single terms, because that is what the reference
/// analyzer produces and a term may only match a whole token.
///
/// Terms that are stop words or shorter than [`MIN_TERM_LENGTH`] are dropped.
pub fn extract_terms(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let lower = text.to_lowercase();
    let bytes = lower.as_bytes();

    let is_head = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    let is_continuation = |byte: u8| is_head(byte) || matches!(byte, b'_' | b'.' | b'-');

    let mut cursor = 0;
    while cursor < bytes.len() {
        if !is_head(bytes[cursor]) {
            cursor += 1;
            continue;
        }
        let start = cursor;
        cursor += 1;
        while cursor < bytes.len() && is_continuation(bytes[cursor]) {
            cursor += 1;
        }
        // Every byte inspected is ASCII, and a multi-byte UTF-8 character has no
        // byte below 0x80, so `start` and `cursor` are always char boundaries.
        push_term(&lower[start..cursor], &mut terms);
    }

    terms
}

/// Keeps a finished token if it is long enough and is not a stop word.
fn push_term(term: &str, terms: &mut Vec<String>) {
    if term.len() >= MIN_TERM_LENGTH && !config::is_stop_word(term) {
        terms.push(term.to_string());
    }
}

/// Normalizes a search query into terms. See [`extract_terms`].
pub fn tokenize(query: &str) -> Vec<String> {
    extract_terms(query)
}

/// Counts searchable tokens in one field of a document.
pub fn count_field(text: &str) -> FieldCounts {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for term in extract_terms(text) {
        *counts.entry(term).or_insert(0) += 1;
    }
    let length = counts.values().sum();
    FieldCounts { counts, length }
}

/// Computes document frequencies and average field lengths for a corpus.
///
/// A term counts once per document toward its document frequency, no matter
/// how many fields of that document it appears in.
pub fn build_stats(documents: &[Vec<FieldCounts>]) -> CorpusStats {
    let field_count = documents.first().map_or(0, Vec::len);
    let mut totals = vec![0usize; field_count];
    let mut document_frequency: HashMap<String, usize> = HashMap::new();

    for document in documents {
        let mut present: HashSet<&str> = HashSet::new();
        for (index, field_counts) in document.iter().enumerate() {
            totals[index] += field_counts.length;
            present.extend(field_counts.counts.keys().map(String::as_str));
        }
        for term in present {
            *document_frequency.entry(term.to_string()).or_insert(0) += 1;
        }
    }

    let document_count = documents.len();
    let average_lengths = totals
        .iter()
        .map(|total| {
            if document_count == 0 {
                0.0
            } else {
                *total as f64 / document_count as f64
            }
        })
        .collect();

    CorpusStats {
        document_count,
        average_lengths,
        document_frequency,
    }
}

/// Smoothed BM25 inverse document frequency for one term.
///
/// The `log(1 + …)` form stays positive for a term present in every document,
/// which the plain ratio would not.
pub fn inverse_document_frequency(document_frequency: usize, document_count: usize) -> f64 {
    let numerator = document_count as f64 - document_frequency as f64 + 0.5;
    let denominator = document_frequency as f64 + 0.5;
    (1.0 + numerator / denominator).ln()
}

/// Scores one document against query terms using BM25F.
///
/// Term frequencies from each field are normalized against that field's own
/// average length, combined under the field's boost, and saturated once, so a
/// term repeated many times cannot dominate the ranking on its own.
pub fn score_document(
    fields: &[Field],
    document: &[FieldCounts],
    stats: &CorpusStats,
    terms: &[String],
) -> f64 {
    if stats.document_count == 0 {
        return 0.0;
    }

    let divisors: Vec<f64> = fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let average = stats.average_lengths.get(index).copied().unwrap_or(0.0);
            let length = document.get(index).map_or(0, |counts| counts.length);
            let ratio = if average != 0.0 {
                length as f64 / average
            } else {
                1.0
            };
            (1.0 - field.length_norm + field.length_norm * ratio).max(MIN_LENGTH_NORM_DIVISOR)
        })
        .collect();

    let mut score = 0.0;
    let mut seen = HashSet::new();
    for term in terms {
        // Repeating a term in the query must not inflate the score, so each
        // distinct term is scored once.
        if !seen.insert(term.as_str()) {
            continue;
        }

        let mut weighted = 0.0;
        for (index, field) in fields.iter().enumerate() {
            let occurrences = document
                .get(index)
                .and_then(|counts| counts.counts.get(term))
                .copied()
                .unwrap_or(0);
            if occurrences > 0 {
                weighted += field.weight * occurrences as f64 / divisors[index];
            }
        }
        if weighted <= 0.0 {
            continue;
        }

        let idf = inverse_document_frequency(
            stats.document_frequency.get(term).copied().unwrap_or(0),
            stats.document_count,
        );
        score += idf * (weighted * (BM25_K1 + 1.0)) / (weighted + BM25_K1);
    }

    score
}

/// Counts how many distinct query terms appear as whole tokens on one line.
pub fn line_hits(line: &str, terms: &[String]) -> usize {
    let tokens: HashSet<String> = extract_terms(line).into_iter().collect();
    terms.iter().filter(|term| tokens.contains(*term)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIELDS: [Field; 3] = [
        Field {
            weight: config::TITLE_BOOST,
            length_norm: config::TITLE_LENGTH_NORM,
        },
        Field {
            weight: config::METADATA_BOOST,
            length_norm: config::METADATA_LENGTH_NORM,
        },
        Field {
            weight: config::BODY_BOOST,
            length_norm: config::BODY_LENGTH_NORM,
        },
    ];

    fn corpus(documents: &[Vec<FieldCounts>]) -> CorpusStats {
        build_stats(documents)
    }

    #[test]
    fn keeps_dots_and_underscores_inside_a_token() {
        assert_eq!(
            extract_terms("Relay.get_target_state"),
            vec!["relay.get_target_state"]
        );
    }

    // The reference analyzer is the regex `[a-z0-9][a-z0-9_.-]*`: once a term
    // starts, `_`, `.` and `-` continue it unconditionally. These cases pin
    // that down, because the intuitive "a trailing dot is punctuation" reading
    // produces different tokens and therefore different scores.
    #[test]
    fn separator_characters_continue_a_token_unconditionally() {
        assert_eq!(extract_terms("release."), vec!["release."]);
        assert_eq!(extract_terms("self._private"), vec!["self._private"]);
        assert_eq!(extract_terms("x..y"), vec!["x..y"]);
        assert_eq!(extract_terms("a_"), vec!["a_"]);
        assert_eq!(extract_terms("__init__"), vec!["init__"]);
    }

    #[test]
    fn a_separator_ends_a_term_when_the_next_char_cannot_start_one() {
        assert_eq!(extract_terms("relay, mode."), vec!["relay", "mode."]);
        assert_eq!(extract_terms("relay- mode"), vec!["relay-", "mode"]);
    }

    #[test]
    fn non_ascii_is_a_term_boundary() {
        assert_eq!(extract_terms("relay → mode"), vec!["relay", "mode"]);
    }

    #[test]
    fn drops_stop_words_and_single_characters() {
        assert_eq!(extract_terms("the of a 1 relay"), vec!["relay"]);
    }

    #[test]
    fn a_hyphenated_identifier_is_one_term() {
        assert_eq!(extract_terms("MNT-238"), vec!["mnt-238"]);
    }

    #[test]
    fn a_stop_word_only_query_yields_no_terms() {
        assert!(tokenize("the of and").is_empty());
    }

    #[test]
    fn idf_stays_positive_for_a_term_in_every_document() {
        assert!(inverse_document_frequency(5, 5) > 0.0);
    }

    #[test]
    fn a_term_counts_once_per_document() {
        let documents = vec![
            vec![count_field("relay"), count_field("relay")],
            vec![count_field("relay"), count_field("pump")],
        ];
        let stats = corpus(&documents);
        assert_eq!(stats.document_frequency.get("relay"), Some(&2));
        assert_eq!(stats.document_frequency.get("pump"), Some(&1));
    }

    #[test]
    fn repeated_query_terms_do_not_double_the_score() {
        let documents = vec![vec![count_field("relay relay"), count_field("relay")]];
        let stats = corpus(&documents);
        let once = score_document(&FIELDS, &documents[0], &stats, &["relay".into()]);
        let twice = score_document(
            &FIELDS,
            &documents[0],
            &stats,
            &["relay".into(), "relay".into()],
        );
        assert_eq!(once, twice);
    }

    #[test]
    fn line_hits_matches_whole_tokens_only() {
        assert_eq!(line_hits("relayed", &["relay".to_string()]), 0);
        assert_eq!(line_hits("the relay", &["relay".to_string()]), 1);
    }

    #[test]
    fn an_empty_corpus_scores_zero() {
        assert_eq!(
            score_document(&FIELDS, &[], &CorpusStats::empty(), &["relay".into()]),
            0.0
        );
    }
}
