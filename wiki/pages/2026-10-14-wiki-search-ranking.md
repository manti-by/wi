---
title: 'Wiki search ranking'
date: '2026-10-14'
type: investigation
status: resolved
tickets: [MNT-238]
tags: [search, bm25, ranking, relevance]
related: [2026-10-14-wi-mcp-server]
---
# Wiki search ranking

## TL;DR

Pages are ranked with BM25F over three separately-weighted zones — title,
metadata, body — with length normalization applied per zone and saturation
applied once at the end.

## Why three zones

A wiki page's most reliable signal is not its body but its title and frontmatter:
those are written by a person summarizing the page, while the body is whatever
the session happened to record. Weighting the title at 10 and the searchable
metadata at 5 against a body weight of 1 reflects that.

The five metadata keys that participate in search are `tags`, `services`,
`areas`, `tickets` and `type`. `title` has its own zone, and `date`, `status`,
`session_id`, `branch` and `related` are display-only — searching for a date or
a branch finds nothing, which is the intent: those fields are navigational, not
topical.

## Why BM25F rather than plain BM25

Plain BM25 concatenates fields into one bag of words, so a term appearing in a
short title counts for exactly as much as the same term appearing fifty times in
a long body. BM25F keeps the fields separate and normalizes each against its own
average length before combining:

```
weighted = Σ field.weight × occurrences / length_normalizer(field)
score   += idf × (weighted × (k1 + 1)) / (weighted + k1)
```

The `÷ length_norm` step is what keeps a chatty page from outranking a precise
one. Title and metadata set `length_norm = 0`, which disables the adjustment
entirely: those fields are short by construction, so normalizing them would only
add noise. The body uses 0.75, close enough to full normalization to matter
without punishing a naturally longer page to zero.

Saturating once at the end is the other half. Applying `k1` per field would let a
term present in all three zones triple-count; applying it to the sum instead means
a term repeated ten times in the body still cannot outweigh a term that appears
in the title.

## Why IDF is smoothed

`idf = log(1 + (N - df + 0.5) / (df + 0.5))`

The textbook form, `log(N / df)`, goes to zero — or negative — for a term present
in every document. The `log(1 + …)` form stays positive, so a term that happens
to appear everywhere still contributes a little rather than actively penalizing
the pages that contain it.

## Why the analyzer is shared

The same function tokenizes documents and queries. That is what makes matching
exact: `relay` matches the token `relay` and not the prefix of `relayed`. A
separate query analyzer that stripped punctuation or stemmed would produce
queries that document text was never tokenized into.

Tokens are runs of `[a-z0-9][a-z0-9_.-]*`. The trailing character class keeps
dots, underscores and hyphens inside a token, because wiki pages are full of API
names (`relay.get_target_state`) and ticket IDs (`MNT-238`) that would otherwise
shatter into meaningless fragments.

## Why results are deterministic

Equal scores break by file name ascending. File names are `YYYY-MM-DD-slug.md`,
so the tie-break is chronological for free, and the same corpus always produces
the same ordering — which is what makes the ranking testable at all.

## Why the corpus is cached by fingerprint

Re-parsing and re-tokenizing every page on every search would dominate the cost
of a query, so the parsed pages, their field counts and the corpus statistics are
cached together and published as one value. The cache is invalidated by
comparing the name, mtime and size of every `*.md` file.

The important property is that items, counts and statistics are published in a
single assignment from a fully built value. A caller can never observe a corpus
whose items come from one generation of the directory and whose statistics come
from another — which would silently produce wrong scores rather than an error.

## Known limits

The fingerprint uses mtime and size, so an edit that preserves both — an
in-place rewrite of equal length within the filesystem's mtime granularity —
would not invalidate the cache. In practice this does not occur on the filesystems
involved; a content hash would cost a full read per query, which is exactly what
the cache exists to avoid.