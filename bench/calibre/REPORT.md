# SpineSprint speed report

Measured binary: SpineSprint 0c6bab2 (e2e feature build). Raw data: data/compare-runs.jsonl.gz, data/compare2-runs.jsonl.gz and data/builds-runs.jsonl.gz. Rerun the tables with summarize.py. The app was called SpineSprint when this was measured, and the commits named here are from before the public history.

## Verdict
- SpineSprint shows first text about 12x faster than Calibre's viewer on a fresh profile and about 6x faster with a reused profile (measured).
- About 7x faster than ebook-edit reaching its file list or a named chapter's source (measured).
- About 300 MB PSS at first text against 670 to 700 MB for Calibre (measured).
- Spelling: no measurable first-text cost, +4 to 7 MB PSS at the mark (measured).
- Drift since step 4: +12 ms (real) and +20 ms (h1p), all between the page's first-text call and its mark; start, webview creation and page load unchanged. Cause unexplained, ff3bdb5 is a plausible origin (inferred, not bisected).

## SpineSprint vs Calibre (median [range] ms from spawn)

| book | app | open | first text | PSS MB at first text | n |
|---|---|---|---|---|---|
| h1p (6,000 ch) | SpineSprint | fresh data dir | 268 [242 to 292] | 301 | 10 |
| h1p | ebook-viewer | fresh profile | 3226 [3078 to 3309] | 687 | 5 |
| h1p | ebook-viewer | reused profile | 1524 [1424 to 1549] | 688 | 5 |
| h1p | ebook-edit | file list | 1824 [1757 to 1911] | 672 | 5 |
| h1p | ebook-edit | chapter named | 1882 [1842 to 1974] | 681 | 5 |
| real (6,193 ch) | SpineSprint | fresh data dir | 261 [246 to 317] | 301 | 10 |
| real | ebook-viewer | fresh profile | 3163 [3040 to 3260] | 701 | 5 |
| real | ebook-viewer | reused profile | 1560 [1441 to 1628] | 700 | 5 |
| real | ebook-edit | file list | 1958 [1830 to 1995] | 680 | 5 |
| real | ebook-edit | chapter named | 1929 [1824 to 1992] | 685 | 5 |
| divbr | SpineSprint | fresh data dir | 276 [260 to 294] | 295 | 10 |
| divbr | ebook-viewer | fresh profile | 3176 [3025 to 3292] (cover, not text) | 688 | 5 |
| divbr | ebook-viewer | reused profile | 1511 [1474 to 1608] (cover, not text) | 689 | 5 |
| divbr | ebook-edit | file list | 1911 [1847 to 1992] | 683 | 5 |
| divbr | ebook-edit | chapter named | 1959 [1854 to 1997] | 683 | 5 |

SpineSprint rows include 12 to 15 ms of screenshot overhead. External first-text frame lands a median 5 ms after the internal ss-first-text mark (range -5 to +20, 20 runs).

## Spelling A/B (internal mark, n=11 each)

| book | 48183bb (pre-spelling) | HEAD |
|---|---|---|
| h1p | 259 [250 to 296] ms, 308 MB | 260 [244 to 282] ms, 315 MB |
| real | 255 [246 to 291] ms, 311 MB | 242 [229 to 275] ms, 315 MB |

The range also contains the shelf and zlib-rs commits, so it is an upper bound on spelling's cost.

## Launch phases, real book, capture off (median ms after spawn)

| phase | 70bb680 (step 4) | 48183bb | HEAD |
|---|---|---|---|
| main | 45.7 | 46.4 | 45.8 |
| setup | 64.5 | 65.2 | 64.9 |
| window_built | 166.4 | 162.5 | 167.6 |
| page_time_origin | 197.1 | 194.0 | 199.4 |
| first_text_called | 221.1 | 228.2 | 212.4 |
| first_text_mark | 230.5 | 255.1 | 242.4 |

## Conditions and limits
- All windows on the hidden SS-TEST output; 68 Calibre and 177 SpineSprint window records, all on SS-TEST. Calibre config, cache and temp isolated in a scratch folder.
- CPU governor on performance, a desktop session with light background load, warm page cache, no drop_caches.
- The builds session (11:02 to 11:13) ran with the screen locked; its internal marks match the unlocked session.
- Real book: SpineSprint opens at a heading-only page, Calibre at a bookmark stored inside the EPUB (META-INF/calibre_bookmarks.txt).
- ebook-edit with no file never shows chapter text; that row is time to file list.
- Not measured: SpineSprint memory after lazy loads finish, Calibre with a long-used config, ebook-edit on a prose chapter of the real book.
