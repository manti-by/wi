//! A cached, immutable view of the parsed wiki corpus.
//!
//! Parsing and tokenizing every page on each search would dominate the cost of
//! a query, so the parsed pages, their field counts and the corpus statistics
//! are cached together and republished as one value. Because the snapshot is
//! built before it is published and never mutated afterwards, a reader can
//! never observe items, counts and statistics from different generations of
//! the directory.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::page::{Page, WIKI_FIELDS, parse_page_file};
use crate::search::{CorpusStats, FieldCounts, build_stats, score_document};

/// Identity of one file: its name, modification time and size.
type FileStamp = (String, u128, u64);

/// One internally consistent snapshot of the pages directory.
#[derive(Debug)]
pub struct Corpus {
    /// The directory the corpus was parsed from.
    pub root: PathBuf,
    /// The file stamps the snapshot was built from. Comparing these against a
    /// fresh fingerprint is what decides whether the snapshot is still valid.
    pub fingerprint: Vec<FileStamp>,
    /// Parsed pages, in file-name order.
    pub items: Vec<Arc<Page>>,
    /// Per-page field counts, aligned to `items` by position.
    pub documents: Vec<Vec<FieldCounts>>,
    /// Corpus-wide statistics derived from `documents`.
    pub stats: CorpusStats,
}

/// One page that matched a query, with its score and the terms it matched.
#[derive(Debug)]
pub struct Ranked {
    /// Index of the page in [`Corpus::items`].
    pub index: usize,
    /// BM25F score; always greater than zero.
    pub score: f64,
    /// The normalized query terms the page was scored against.
    pub terms: Vec<String>,
}

/// Single-slot cache that rebuilds a corpus only when its files change.
#[derive(Debug, Default)]
pub struct CorpusCache {
    cached: Mutex<Option<Arc<Corpus>>>,
}

impl CorpusCache {
    /// Creates an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the current snapshot for a directory, rebuilding it if stale.
    ///
    /// The lock is held across the rebuild so two concurrent searches cannot
    /// both parse the directory; everything inside it is in-memory plus a few
    /// small local reads.
    pub fn get(&self, root: &Path) -> Arc<Corpus> {
        let mut cached = self.cached.lock().expect("corpus cache poisoned");
        let fingerprint = fingerprint_pages(root);

        if let Some(corpus) = cached.as_ref()
            && corpus.root == root
            && corpus.fingerprint == fingerprint
        {
            return Arc::clone(corpus);
        }

        Arc::clone(cached.insert(build_corpus(root, fingerprint)))
    }
}

/// Builds a fresh corpus from a directory, skipping unusable pages.
fn build_corpus(root: &Path, fingerprint: Vec<FileStamp>) -> Arc<Corpus> {
    let mut items = Vec::new();
    let mut documents = Vec::new();

    let mut paths = markdown_files(root);
    paths.sort();
    for path in paths {
        if let Some(page) = parse_page_file(&path) {
            documents.push(page.fields());
            items.push(Arc::new(page));
        }
    }

    let stats = build_stats(&documents);
    Arc::new(Corpus {
        root: root.to_path_buf(),
        fingerprint,
        items,
        documents,
        stats,
    })
}

/// Lists the `*.md` files directly under a directory. Non-recursive, matching
/// the reference implementation: the pages directory holds pages, not trees.
fn markdown_files(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .filter(|path| path.is_file())
        .collect()
}

/// Describes every page file by name, modification time and size.
///
/// A directory that cannot be read fingerprints as empty, which is what makes
/// "the directory went away" indistinguishable from "it is empty" — both mean
/// there is nothing to serve.
fn fingerprint_pages(root: &Path) -> Vec<FileStamp> {
    let mut stamps = Vec::new();
    for path in markdown_files(root) {
        let Ok(metadata) = path.metadata() else {
            continue;
        };
        let Some(name) = path.file_name() else {
            continue;
        };
        stamps.push((
            name.to_string_lossy().into_owned(),
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |elapsed| elapsed.as_nanos()),
            metadata.len(),
        ));
    }
    stamps.sort();
    stamps
}

/// Scores every page against the query terms and returns the matches, best
/// first. Ties keep corpus order, which is file-name order.
pub fn rank(corpus: &Corpus, terms: &[String]) -> Vec<Ranked> {
    if terms.is_empty() || corpus.stats.document_count == 0 {
        return Vec::new();
    }

    let mut ranked: Vec<Ranked> = corpus
        .items
        .iter()
        .zip(&corpus.documents)
        .enumerate()
        .filter_map(|(index, (_, document))| {
            let score = score_document(&WIKI_FIELDS, document, &corpus.stats, terms);
            (score > 0.0).then(|| Ranked {
                index,
                score,
                terms: terms.to_vec(),
            })
        })
        .collect();

    // `sort_by` is stable, so equal scores stay in file-name order.
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("wi-corpus-{}-{tag}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(root: &Path, name: &str, contents: &str) {
        std::fs::write(root.join(name), contents).unwrap();
    }

    #[test]
    fn loads_pages_in_file_name_order() {
        let root = make_root("order");
        write(&root, "b.md", "---\ntitle: Beta\n---\nbody b");
        write(&root, "a.md", "---\ntitle: Alpha\n---\nbody a");
        let corpus = CorpusCache::new().get(&root);
        let names: Vec<&str> = corpus.items.iter().map(|page| page.name.as_str()).collect();
        assert_eq!(names, vec!["a.md", "b.md"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_missing_directory_yields_an_empty_corpus() {
        let root = make_root("missing").join("absent");
        let corpus = CorpusCache::new().get(&root);
        assert_eq!(corpus.stats.document_count, 0);
        assert!(corpus.items.is_empty());
    }

    #[test]
    fn items_documents_and_stats_always_agree() {
        let root = make_root("agree");
        write(&root, "relay.md", "---\ntitle: Relay\n---\nthe relay mode");
        write(&root, "pump.md", "---\ntitle: Pump\n---\na pump page");
        let corpus = CorpusCache::new().get(&root);
        assert_eq!(corpus.items.len(), corpus.documents.len());
        assert_eq!(corpus.items.len(), corpus.stats.document_count);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rank_returns_only_positive_scores() {
        let root = make_root("rank");
        write(&root, "relay.md", "---\ntitle: Relay\n---\nthe relay mode");
        write(&root, "pump.md", "---\ntitle: Pump\n---\na pump page");
        let cache = CorpusCache::new();
        let corpus = cache.get(&root);
        let hits = rank(&corpus, &["relay".to_string()]);
        assert_eq!(hits.len(), 1);
        assert_eq!(corpus.items[hits[0].index].name, "relay.md");
        assert!(rank(&corpus, &[]).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_new_page_invalidates_the_cached_corpus() {
        let root = make_root("invalidate");
        write(&root, "relay.md", "---\ntitle: Relay\n---\nrelay");
        let cache = CorpusCache::new();
        assert_eq!(cache.get(&root).items.len(), 1);
        write(&root, "pump.md", "---\ntitle: Pump\n---\npump");
        assert_eq!(cache.get(&root).items.len(), 2);
        let _ = std::fs::remove_dir_all(root);
    }
}
