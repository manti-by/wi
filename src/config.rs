//! Tunables that mirror the reference `wi` search configuration.
//!
//! Every constant here has a counterpart in the Python implementation this
//! binary replaces. They are grouped in one module so a future divergence is a
//! one-file diff rather than a hunt through the ranking code.

/// Default number of search results returned when the caller does not ask.
pub const DEFAULT_LIMIT: usize = 5;

/// Upper bound on search results, applied after clamping the caller's `limit`.
pub const MAX_RESULTS: usize = 20;

/// Longest accepted search query, in characters.
pub const MAX_QUERY_LENGTH: usize = 500;

/// How many body lines a single search hit may quote.
pub const MAX_SNIPPETS: usize = 3;

/// Character cap applied to each quoted snippet line.
pub const SNIPPET_LENGTH: usize = 200;

/// Shortest token kept by the analyzer; single characters are dropped.
pub const MIN_TERM_LENGTH: usize = 2;

/// BM25 term-frequency saturation parameter.
pub const BM25_K1: f64 = 1.2;

/// Boost applied to a page's frontmatter `title` zone.
pub const TITLE_BOOST: f64 = 10.0;

/// Length normalization for the title zone; zero disables it.
pub const TITLE_LENGTH_NORM: f64 = 0.0;

/// Boost applied to a page's searchable metadata zone.
pub const METADATA_BOOST: f64 = 5.0;

/// Length normalization for the metadata zone; zero disables it.
pub const METADATA_LENGTH_NORM: f64 = 0.0;

/// Boost applied to a page's body zone.
pub const BODY_BOOST: f64 = 1.0;

/// Length normalization for the body zone.
pub const BODY_LENGTH_NORM: f64 = 0.75;

/// Frontmatter keys whose values are flattened into the searchable metadata
/// zone. Deliberately excludes `title`, `date` and `status`: `title` has its
/// own boosted zone and the others are display-only.
pub const METADATA_KEYS: [&str; 5] = ["tags", "services", "areas", "tickets", "type"];

/// Frontmatter keys rendered, in order, by the per-page metadata summary.
pub const SUMMARY_KEYS: [&str; 4] = ["services", "areas", "tags", "tickets"];

/// Stop words dropped by the analyzer.
///
/// A query made only of these yields no terms, which is reported as "no
/// matches" rather than as an error.
pub const STOP_WORDS: [&str; 26] = [
    "a", "an", "and", "are", "be", "been", "did", "do", "does", "for", "how", "in", "is", "it",
    "of", "on", "or", "that", "the", "this", "to", "was", "were", "what", "why", "with",
];

/// Returns `true` when a token is a stop word.
pub fn is_stop_word(term: &str) -> bool {
    STOP_WORDS.contains(&term)
}
