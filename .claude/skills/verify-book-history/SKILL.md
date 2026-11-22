---
name: verify-book-history
description: Model-check Jadeslip's book history (the append-only log in crates/core/src/history.rs, how Book::commit and take_snapshot write to it, how landed/state_at/restore_point rebuild a past version, and how history cleanup drops points and stores older versions as differences) with TLA+/TLC across crashes, power cuts, failed flushes and outside edits. Use before changing the history log format, History::stage/scan_from/blob, landed or state_at, restore_point, history cleanup or compaction (history/prune.rs), or the history steps in Book::commit, when a change touches a function listed under Coverage, or when asked whether "restore to this point" can bring back the wrong version or lose a point, or whether compaction can lose a version or make one unreadable.
---

# Verify the book history

Every save appends the old text of each changed file to `<data>/books/<key>/history/log`, followed by a record (`Record`: `file_before`, `file_after`, and the old blob key per changed entry). The History panel lists points from `landed()`, and "Restore" rebuilds the book as it was before a point with `Landed::state_at` and `Book::restore_point`. History cleanup (`History::prune`) rewrites the log: it drops old automatic points and stores each older chapter version as the difference from the next newer version of the same chapter (a TEXT frame that names its base), so reading such a version also needs its base. A wrong rebuild silently puts the wrong chapter text into the user's book. A lost point removes a way back the user was promised. Ordinary tests cover particular sequences. This check runs every order of saves, snapshots, crashes, power cuts, failed history flushes and outside edits, within the bounds below.

The save and backup protocol itself (ledger, rename, recovery) is checked by the `verify-book-commit` skill, not here.

## The rules it checks

| Invariant (in `model/BookHistory.tla`) | Rule in plain words | Harm if broken |
|---|---|---|
| `VersionExact` | For every point the panel shows, each chapter version the history claims for that point is the chapter's real text at that point, and reading that version back gives exactly that text or fails as damaged. | Restoring a chapter puts back text it never had. |
| `RestoreExact` | When a point is offered as a whole-book restore (`complete`), the rebuilt book equals the book as it really was at that point. | Whole-book restore silently produces a mix of versions. |
| `Restorable` | Every version that a point needs, from saves that finished without a warning, can be read back (`History::blob` succeeds). A version stored as a difference reads back only if its base does too. | Restore fails with "This saved version is damaged". |
| `NoPointLost` | A save or named snapshot that finished without a warning stays in the panel (as itself or as an equal state), unless the book is now in exactly that state, or a history cleanup was allowed to drop it (an automatic save older than the cutoff and not the newest point). | A point the user saw, and relied on, vanishes. |
| `NoFalseGap` | With no outside edits and no failed flushes, crashes and power cuts never make a point "incomplete". | Whole-book restore is refused for no reason. |
| `HeldExact` | A point id that a History panel got earlier, before saves, crashes or a cleanup in another process, names that same point in every later view, or no point. | "Restore to this point" brings back a different point. |
| `HeldFound` | While the point that id named is still in history, the id still finds it. | A point still listed elsewhere is refused as gone. |

Rules are checked from two views: the open window after it refreshes its index, and a freshly opened window that scans the log from the start.

## Run

```
.claude/skills/verify-book-history/scripts/run.sh            drift check, then every config (about 6 minutes)
.claude/skills/verify-book-history/scripts/run.sh drift      only: has covered code changed since the model was confirmed?
.claude/skills/verify-book-history/scripts/run.sh checks     only the configs that must pass
```

Requirements: Java 11 or newer, Python 3, and TLC 1.7.4 (`tla2tools.jar`, sha256 `936a2620…0e88`), found the same way as in `verify-book-commit`: `$TLA2TOOLS`, `mise where http:tla2tools@1.7.4`, or `~/.local/share/mise/installs/http-tla2tools/1.7.4/`. If none is found, it prints the download URL.

TLC 1.7.4 sometimes throws `ArithmeticException: Division by zero` in its end-of-run statistics, after the search has finished. `run.sh` counts that run as `no violation` only when the log also says `0 states left on queue` and names no violated invariant. Any other TLC exception shows as `TLC error, see log` and is a real problem in the spec.

Logs, with full TLC traces, go to `target/verify-book-history/<time>/<group>-<name>.log`. The script ends with `run: as expected` and exit 0, or `run: SURPRISE` and exit 1.

Read every TLC result with the pstack `/statechart` skill. On a violation it shows the shortest path to the broken rule in plain words. When a check passes, it draws how saves move between states (these configs exceed its chart budget, so give it a smaller config to see the whole graph). Give it `model/BookHistory.tla` and the config you ran.

## Reading a result

- `checks/*` must give `PASS`. A `FAIL` means a rule broke within that config's scope.
- `findings/*` are known violations in the current code and must give `REPRODUCES`. `CHANGED` means a finding no longer reproduces or now breaks a different rule. That is good news if the code was fixed on purpose: move the config into `checks/`, give it the full INVARIANTS list, and update the Findings section.
- `mutants/*` are deliberately broken variants and must give `CAUGHT`. They prove each rule can fail. `MISSED` means a rule or the model lost its teeth, usually after a model edit.

Real bug or modeling error? Turn the trace into code steps with the Coverage table, then check each step against the code:
- If every step is something the code really does (the same order, the same check or lack of one, the same thing kept in the window's memory versus read from the file), it is a real bug. Report it as steps in the code's words. Do not weaken the rule, and do not change the model to make it pass. Replay it as a scratch test in `crates/core/src/commit/tests.rs` (`fail_at(Step::…)` injects a failed flush; overwriting bytes of the log file stands in for lost pages) before reporting it.
- If one step is impossible in the code, it is a modeling error. Fix that action to match the code and say which line the old action got wrong.

## Scope, bounds and assumptions

What is checked: one Jadeslip window on one book, up to 4 history appends (saves plus named snapshots; 3 in the failed-flush configs), and the failures each config allows.
- `checks/crashes`: up to 2 crashes (including in the middle of writing a history entry) and 1 outside edit. All five rules. About 0.8 M states.
- `checks/power-cut`: 1 power cut, 1 crash, 1 outside edit. All five rules. About 1.1 M states.
- `checks/torn-power-cut`: as `power-cut`, but a power cut may also bring back unflushed history bytes damaged. All five rules. About 2.3 M states.
- `checks/failed-flush`: 1 failed history flush, 1 power cut that may tear, 1 outside edit. All five rules. About 0.14 M states.
- `checks/failed-flush-after-crash`: as `failed-flush` with 1 crash and no tearing. Every rule except `Restorable`, which the Finding breaks here. About 0.5 M states.
- `checks/prune`: 1 history cleanup (in the open window or on a closed book, any cutoff, with compaction), 1 crash, 1 outside edit, 4 appends. All five rules. About 1.6 M states.
- `checks/prune-power-cut`: 1 cleanup, 1 crash, 1 power cut that may tear, 1 outside edit, 3 appends. All five rules. About 1.5 M states.
- `checks/prune-failed-flush`: 1 cleanup, 1 crash, 1 power cut, 1 failed flush, 3 appends. Every rule except `Restorable` (the Finding). About 0.13 M states.
- `checks/prune-verify`: as `prune`, with the cleanup's plan folding across gaps (`fold_gaps_verified`). It must pass, which proves `Plan::verify` refuses a wrong plan. About 1.6 M states.
- `checks/prune-twice`: 2 cleanups, so the second one meets differences the first one wrote, 1 outside edit, 4 appends. All five rules. About 0.32 M states.
- `checks/prune-twice-crash`: as `prune-twice` with 1 crash and no outside edit. All five rules. About 0.14 M states. Two cleanups with both a crash and an outside edit also pass (about 3.0 M states) but take over 5 minutes, so they are split into these two.
- `checks/compact-verify-unwritten-base` (4 appends, 1 cleanup, about 0.16 M states), `checks/compact-verify-wrong-base` (3 appends, 1 cleanup, about 0.014 M states) and `checks/compact-verify-stale-base` (4 appends, 2 cleanups, about 0.29 M states): 1 outside edit each, with a compaction bug turned on and `Plan::verify` on. All five rules. They must pass, which proves `Plan::verify` refuses each bad log (see the compaction mutants under Findings).
- `checks/held-point`: as `prune` with 3 appends, and a History panel that holds one point's id from any idle moment on (`Hold`). All seven rules. About 0.46 M states.
- `checks/id-from-after-verified`: 3 appends, 1 cleanup, 1 outside edit, 1 held id, with the id built from `file_after` (`id_from_after_verified`) and `Plan::verify` on. All seven rules. About 0.06 M states. It must pass, which proves `Plan::verify` refuses a cleanup that would change a kept point's id.

The model abstracts and assumes:
- The book has two entries, `a` (two possible texts) and `b` (absent or one text that equals one of `a`'s). A blob key depends only on the bytes (`BlobKey::of`), so the same text in two entries, or the same text again later, shares one stored blob. A file hash is the whole content (SHA-256 is taken as collision-free), and `same_as` (crc32 plus length) is taken as exact.
- A record's `file_before` (`slot.ledger.head`) is the hash of the head reader that the save diffs against. This rests on `ensure_slot`/`find_valid` setting `head` from `state.hash`, the hash of that reader's file as it was opened or last saved (the first backup copy is checked against it), `Slot::settle` updating the in-memory head before it writes, and `Book::commit` dropping a slot with a pending head. `verify-book-commit` checks the ledger itself.
- Log frames are whole units. A crash in the middle of `History::stage` leaves any prefix of its frames followed by a partial frame. `read_frame` rejects a frame whose header, kind, length or CRC is bad, and `scan_from` then calls `next_frame`, which searches for the next `MAGIC` that starts a valid frame. With none, the scan stops and the next `stage` cuts the log there (`set_len(end)`). A false `MAGIC` that also passes the CRC is taken as impossible.
- A failed history fsync (`Staged::sync`, or the `Step::HistorySynced` hook after it) loses any non-empty subset of the unflushed frames on disk while the page cache still serves them, until the system evicts them (`Evict`). `Staged::finish` then cuts the log back to where that stage started (`set_len(start)`), so this stage's frames are gone at once. Unflushed frames from before the stage (left by a crashed save) stay. The cut is treated as durable: if a power cut brings the cut frames back, they come back damaged, and the scan skips them. Later successful fsyncs do not rewrite lost pages.
- A power cut keeps flushed frames, brings lost frames back bad, and may cut the unflushed tail anywhere. With `TornUnsynced = TRUE`, each unflushed frame it keeps may also come back bad. That depends on the filesystem: ext4 `data=ordered` makes it unlikely.
- The book's folder fsync always succeeds (its failures are in `verify-book-commit`). After a power cut, the book is any version renamed since the last folder sync. `Open` fsyncs the folder, as `recover_locked` does. Outside edits are durable and change the `FileStamp`.
- History cleanup (`Prune`) reads the log as `History::prune` does, plans with `plan` (keep named points, points at or after the cutoff and the newest point; fold each dropped point into the kept point before it with first-seen-wins, and stop folding at a gap), compacts (next item), writes each cited version once (its recomputed TEXT frame, or else its old frame copied as is when that frame is intact and the version is not `lost`), writes a new log whose frames are all flushed (`sync_data` before the rename), and replaces the log only if `Plan::verify` finds every kept point rebuilds exactly as before and every version the kept points cite reads back the same text, or the same damage, from the new log as from the old, with no written version unreadable (`SameTexts`). A cleanup goes ahead when it drops a point or compaction recomputed a version. Cutoffs are point ids, since time grows with them. The rename of the new log is durable only once the history folder is synced: the cleanup's own `sync_dir` may fail (it is ignored), so until a later history save syncs the folder (`Staged::sync` now does on every save), a power cut may bring back the old log (`pending`, `oldLog`). A cleanup on a closed book first syncs the book's folder (`prune_closed`), so it decides from a durable book. A dropped point that never finished without a warning, folded into a kept point, makes that kept point untrusted for `Restorable` (`untrusted`); restore results stay the same, which `Checked` confirms.
- Compaction (`Compact`) follows `Plan::compact`. `links` walks the kept records newest first and gives each cited version, once, the next newer version of the same chapter as its planned base; within one record it takes chapter `a` before `b`. `limit_chains` drops a base that no earlier link holds. In that order, a version whose text reads back is placed; its base counts only if that base was placed before it. It gets a new TEXT frame when its stored base differs from that base: a difference from the base, or a full copy. The 10% saving rule is a free choice (`Squeezable`), which covers every outcome the rule can pick. A version stored whole (Packed, or a TEXT frame with no base) keeps its frame unless a difference is kept. An unreadable TEXT version goes to `lost`. Deflate never fails, so the branch that keeps an old difference unplaced when no new body can be made is left out.
- A TEXT frame stores its key, its base and the text its difference was computed from (`from`). Reading a version (`ReadBack`, which stands for `History::stored` → `text_within` → `rebuilt`) follows bases through the index, at most `MaxChain` (3) steps. It gives the key's text only if every frame on the way is intact and each base reads back as the text that frame was computed from. Otherwise it gives damage: this is the crc32 and SHA-256 check of the rebuilt text, with SHA-256 taken as collision-free. Because a key is its text, the `newer` shortcut in `Newest::read` gives the same results and is not modelled separately.
- `Prune` is one step. Pages evicted while a cleanup runs can only make `Plan::verify` refuse, because it reads both logs again at the end.
- A power cut can bring back only the log from before the latest cleanup (`oldLog`), so configs with power cuts allow one cleanup.
- Revert to backup and `restore_point` are saves to some content, so `Stage(c)` with any `c` covers them. The way `restore_point` picks blobs is checked through `Restored` in `RestoreExact`.
- A point's id (`Handle`, `Identity`) is `Record::id`: a hash of the record's kind and name, its time, `file_before` and the names of the chapters it changed, all fields that `fold` and the renumbering of `Restore { to }` leave alone. `history_changes`, `history_diff` and `restore_point` look it up with `Landed::find` in the log as it is now, so a held id is checked against every later view. The model has no clock, so a record's time is its own number (`rec.id`): two records with the same kind, start and chapters written in the same millisecond are left out. A held id lives in the frontend, so crashes, power cuts and cleanups leave it in place.

Left out: chains longer than one step. A chapter has two texts here, so a chain never reaches `CHAIN` (a full copy at least every 20 steps, `limit_chains`) or `MAX_CHAIN` (64); the unit tests in `crates/core/src/history/tests.rs` cover those, among them `damage_to_the_newest_version_reaches_at_most_twenty_versions_back` and `versions_that_name_each_other_as_base_read_as_damaged_and_cleanup_still_works`. Also left out: the byte format of differences (`history/text.rs`); a dry run (`dry_run` skips compaction and writes nothing); a failed `set_len` in `Staged::finish` (the original error is still returned); a second window (its incremental index after another window truncates a torn tail); the byte layout of frames (`frame`, `le32`) beyond "a damaged frame is detected"; `history_diff` text; `unwrapped` link repair for deleted chapters; `rows`/`spine_delta`; record timestamps; Windows; more appends or failures than the bounds above.

## Findings

The three findings from the first version of this model were fixed on 2026-11-21, and their configs are now `checks/failed-flush` and `checks/torn-power-cut`. They were: a failed history flush hid and then deleted later good saves; a version lost in a failed flush was reused by a later save; a version torn by a power cut was reused the same way. The tests `a_failed_snapshot_flush_does_not_cost_the_saves_after_it`, `a_version_lost_in_a_failed_flush_is_stored_again_by_a_later_save`, `a_version_torn_by_a_power_cut_is_stored_again_by_the_next_save` and `damage_in_the_middle_of_the_history_keeps_the_saves_after_it` in `crates/core/src/commit/tests.rs` replay them against the real code. The fix has three parts, each with a mutant that brings a bug back: `Staged::finish` cuts the log back after a failed flush (`keep_failed`), `read_frame` checks blob CRCs (`no_blob_crc`), and `scan_from` skips past a damaged frame (`stop_at_damage`). With the skip in place, the cut to `end` in `History::stage` no longer guards a rule, so it has no mutant.

1. **A crashed save's version, lost in a later failed flush, is reused** (`findings/crash-then-failed-flush-reuses-lost-version`, `Restorable`). A save writes its history frames and the process dies before the book is renamed, so those frames stay in the log unflushed. After a reopen, a named snapshot's history fsync fails, and the lost pages include the crashed save's blob, which lies before the snapshot's own start, so `Staged::finish` does not cut it. While the page cache still serves that blob it reads back intact, so a later save of the same text dedupes against it in `History::stage`. After the system evicts the pages, or a power cut, restoring to before that later, warning-free save fails with "This saved version is damaged, so it cannot be used." No read can tell a lost page apart while the cache still serves it, so a fix needs to know which frames were ever flushed: for example, fsync the log when a window first opens it, and on a failed flush stop deduping against every frame not known to be flushed. Restore never gives back the wrong text here.

Two bugs in the history cleanup were found and fixed when the cleanup was added (2026-11-22). Each fix has a mutant that brings the bug back:
- A failed or skipped folder sync after the cleanup's rename let a power cut bring back the old log, losing every save made after the cleanup. `Staged::sync` now syncs the history folder on every save (`no_dir_sync`).
- A cleanup on a closed book after a crash trusted a book rename that was not yet durable and dropped the points that a power cut would then need. `prune_closed` now syncs the book's folder first (`closed_no_sync`).

The cleanup's own guards each have a mutant too, with `Plan::verify` switched off so the rules see the bad plan: `fold_gaps`, `fold_last`, `drop_named`, `drop_newest`, `prune_skip_blobs` and `prune_no_sync`.

Compaction (2026-11-22) has the same kind of mutants, each with `Plan::verify` switched off, and each breaks `Restorable`:
- `unwritten_base`: bases come from every record before folding, so a version is stored as a difference from a version that only dropped points cite, which the cleanup never writes.
- `wrong_base`: a difference names its base but was computed against another text.
- `stale_base`: a stored difference is always copied as is, so after a second cleanup drops its base, the base is gone.

Each has a twin, `checks/compact-verify-unwritten-base`, `-wrong-base` and `-stale-base`, that turns the same bug on (`*_verified`) with `Plan::verify` switched on. The twins must pass, which proves `Plan::verify` refuses every one of these logs. `verify_skips_texts` is `unwritten_base` with `Plan::verify` comparing points but not texts; it is caught, so the text comparison is what refuses it. `wrong_base_unchecked` is `wrong_base` with the text check in `rebuilt` skipped; it breaks `VersionExact`, so that check is what turns a wrong base into "damaged" instead of the wrong text.

A point's id was its position in the log until 2026-11-22. A cleanup in another process renumbers the log, so a History panel listed before it restored, listed and diffed a different point. `commit/tests.rs` replays it in `a_point_listed_before_a_cleanup_elsewhere_restores_that_point`. The id is now `Record::id`, and two mutants bring the bug back:
- `position_ids`: the id is the record's position. It breaks `HeldExact` (save, hold it, snapshot, a cleanup drops the save, and the held id now names the snapshot).
- `id_from_after`: the id also covers `file_after`, which `fold` changes, with `Plan::verify` switched off. It breaks `HeldFound`. Its twin `checks/id-from-after-verified` passes, because `Plan::verify` compares every kept point's id.

## Coverage

The model was last confirmed against the code at **commit `ba47b58`** (2026-11-22). `coverage.json` stores a hash of each covered function body at that commit. `run.sh drift` names each function whose text has changed since.

| Model action or operator | Code |
|---|---|
| `Open` | `Book::open_with`: a fresh `History::new` (empty index) and `recover` (fsync of the book's folder) |
| `Stage` | `Book::commit`: `ensure_slot`, `file_before = slot.ledger.head`, `history::start` with `rewritten_names` (or `diffed_names` for a restore) and `Cause::record`, into `History::stage`: `refresh`, dedupe by `blob_meta`, `set_len(end)`, two `write_all` halves around `Step::HistoryWriting` |
| crash branch of `Stage` | a process dying between the two halves, or between frames |
| `SyncHistory`, `AfterFail` | `Syncing::spawn` → `Staged::sync` (`sync_data`, folder syncs on first create), `Syncing::finish` → `Staged::finish` (`Step::HistorySynced`; on failure `set_len(start)`); failure becomes `history_warning` and the save goes on |
| `Rename` | the content hash compare and the `FileStamp` recheck, then `fs::rename` in `Book::commit`; a changed hash or stamp returns `ChangedOnDisk` and the staged record stays in the log |
| `Finish` | `sync_dir` after the rename, `slot.settle`, `state.hash = Some(hash)` |
| `Snapshot` | `Book::take_snapshot` → `History::append` (stage, `Staged::sync`, `Staged::finish`; a failure cuts the frames and is returned as an error) |
| `Crash`, `PowerCut`, `Evict`, `OutsideWrite` | a process dying; power loss; the kernel dropping pages whose writeback failed; another program replacing the book |
| `Intact`, `ScanFrom`, `Refresh` | `read_frame` (header, kind, length and CRC of blobs and records), `scan_from` with `next_frame` to skip damage, `History::refresh` (reset when `end > len`) |
| `Indexed`, `ReadBack`, `ReadText`, `Readable` | `History::blob` → `History::stored` → `text_within`: frame at the indexed offset, CRC and key checked (`checked_payload`, `packed_from`), bases followed through the index, each difference rebuilt and its text checked (`rebuilt`); `History::text_from`, `History::readable` |
| `PopFor`, `Build`, `Points` | `landed`: the pop loop per record, then the tail pop against `current` (`Book::file_hash`) |
| `Walk`, `StateAt` | `Landed::state_at`: `or_insert` of each `before`, stop at the first gap, `complete = !gap_at_end` |
| `Prune`, `Plan`, `Folded`, `Rewrite`, `CopyOf`, `Checked` | `Book::prune_history` (in-window, `file_hash` refuses a changed book) or `storage::prune_closed` (closed book, folder sync first), into `History::prune`: `plan`, `fold`, `Plan::compact`, the early return when nothing is dropped and `texts` is empty, `Plan::write` (`texts` or `History::copy_of`, `sync_data`), `Plan::verify`, `replace_with` (rename), then a best-effort `sync_dir` |
| `Links`, `Squeezable`, `Compact`, `CompactFrom`, `Recompute`, `NewText` | `links`, `limit_chains`, `Plan::compact` (with `Newest::read`) |
| `SameTexts` | the text loop at the end of `Plan::verify`: each link's text from the old log and the new (`Newest::read`), and `History::readable` for a written version |
| `Restored` | `Book::restore_point(WholeBook)`: known entries from their blobs, the rest kept; `StateAt::version` for one chapter |
| `Hold`, `Identity`, `Handle` | `Book::history` and `Book::take_snapshot` hand out `Record::id`; `Book::state_at` turns it back into a position with `Landed::find` before `Landed::state_at` |
| the `Identity` line of `Checked` | `was.id() == now.id()` in `Plan::verify` |

The covered functions are listed in `scripts/coverage.py` (`COVERED`, 52 functions; `coverage.py list` prints them).

## Does my change touch this?

Run `run.sh drift`. If it reports none, the model still describes the code; run `run.sh checks` if you changed nearby behaviour, and you are done. These also count even when drift misses them: a change to the frame format or `MAGIC`, to which `Step` the history writes hit, to the order of history staging, sync and rename in `Book::commit`, a new caller that appends to the history log or rebuilds a past version without going through `landed` and `state_at`, or a point id handed out as anything other than `Record::id`. The model lacks those actions.

## When the covered code changed

1. Run `git diff <the commit named under Coverage> -- crates/core/src/history.rs crates/core/src/commit/history.rs crates/core/src/commit.rs crates/core/src/commit/draft.rs crates/core/src/backup.rs crates/core/src/history/prune.rs crates/core/src/storage.rs` and read only the functions that drift named.
2. For each changed function, find its rows in the Coverage table and decide: does the model still do exactly what the code now does, step for step, with the same indivisible steps?
3. If yes, the change is invisible at this level (renaming, logging, error text). Run `run.sh` and go to step 6.
4. If no, edit the matching action or operator in `model/BookHistory.tla` to mirror the new code. Do not touch the invariants or the `Mutant` branches. Run `run.sh`. Then:
   - a `checks` FAIL: read the trace (see Reading a result). If the change was meant to fix a Finding, its config should now say `CHANGED`; move it to `checks/`.
   - a mutant MISSED: your edit probably bypassed the step the mutant breaks. Make the mutant's `Mutant = "…"` branch still break that step.
5. A new step that writes, syncs, truncates or reads the log gets its own action or operator, a row in the Coverage table and, if it guards a rule, a mutant config under `mutants/` that removes it and must be `CAUGHT`.
6. Only after `run.sh` says `run: as expected` and you have checked the model against the new code, run `python3 .claude/skills/verify-book-history/scripts/coverage.py stamp`. Then update the commit and date in the Coverage section to the new HEAD. Never stamp just to silence drift.

The repo rule of no comments in any file applies here too. The mapping lives in this file, not in the spec.
