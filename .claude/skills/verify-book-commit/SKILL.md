---
name: verify-book-commit
description: Model-check Jadeslip's book save and backup protocol (Book::commit, recovery, the backup ledger in crates/core/src/commit.rs and backup.rs) with TLA+/TLC across crashes, power cuts, two windows and outside edits. Use before changing anything in the save, revert, backup or recovery path, when a change touches a function listed under Coverage, or when asked whether a save can lose the user's book or its backup.
---

# Verify the book save and backup protocol

Saving an edit rewrites the user's EPUB in place. Every save path, including the edit save, replace across book, chapter manage, snapshot restore and Revert, ends in `Book::commit` (`crates/core/src/commit.rs`). The first save also makes a backup of the book as it was (`crates/core/src/backup.rs`), and "Revert to backup" relies on finding that backup again. Ordinary tests check particular crash points. This check runs every order of steps that two Jadeslip windows, a process crash, a power cut and an outside program can produce, within the bounds below.

## The rules it checks

| Invariant (in `model/BookCommit.tla`) | Rule in plain words | Harm if broken |
|---|---|---|
| `BookNeverTorn` | After any crash or power cut, the book file holds a whole version: the old one or the new one. | The user's book is corrupt and will not open. |
| `BackupBeforeLedger` | A backup record (`ledger.json`) never exists without its backup copy (`book.epub`) beside it. | Revert says a backup exists, then fails. |
| `RevertReachable` | When nothing is mid-save and no outside program has touched the book, the backup that `find_valid` picks for the current file is the copy of the book as Jadeslip first saw it. | "Revert to backup" silently disappears, or restores the wrong version, after a crash. |
| `NoMissedExternalWrite` | Jadeslip never replaces a book file that another program had changed before Jadeslip's last check. | Edits made in another program are silently overwritten. |
| `NoExternalWriteOverwritten` | Stronger: Jadeslip never replaces a file that another program changed at any point after Jadeslip read it. | Same as above. Only the findings config checks it (see Findings). |

## Run

```
.claude/skills/verify-book-commit/scripts/run.sh            drift check, then every config (about 15 s)
.claude/skills/verify-book-commit/scripts/run.sh drift      only: has covered code changed since the model was confirmed?
.claude/skills/verify-book-commit/scripts/run.sh checks     only the configs that must pass
```

Requirements: Java 11 or newer, Python 3, and TLC 1.7.4 (`tla2tools.jar`, sha256 `936a2620…0e88`). `run.sh` finds TLC through `$TLA2TOOLS`, `mise where http:tla2tools@1.7.4`, or `~/.local/share/mise/installs/http-tla2tools/1.7.4/`, and checks the hash. If none is found, it prints the download URL.

Run it from anywhere in the repo. Logs, including the full TLC traces, go to `target/verify-book-commit/<time>/<group>-<name>.log`. The script ends with `run: as expected` and exit 0, or with `run: SURPRISE` and exit 1.

Read every TLC result with the pstack `/statechart` skill. On a violation, it shows the shortest path to the broken rule in plain words. When a check passes, it draws how saves move between states. Give it `model/BookCommit.tla` and the config that you ran.

## Reading a result

Each line is one config:

- `checks/*`, which must give `PASS`. A `FAIL` means a rule broke within that config's scope.
- `findings/*`, which are known violations in the current code, must give `REPRODUCES`. `CHANGED` means the finding no longer reproduces, or now breaks a different rule. That is good news if the code was fixed on purpose. Then move the config into `checks/`, change its INVARIANTS to the full set, and update the Findings section.
- `mutants/*`, which are deliberately broken orders, must give `CAUGHT`. They prove each rule can fail. `MISSED` means a rule or the model lost its teeth, usually after a model edit.

Real bug or modeling error? Turn the trace into code steps with the table under Coverage, then check each step against the code:
- If every step is something the code really allows (the same order, the same check or no check, the same thing kept in memory versus on disk), it is a real bug. Report it as a sequence of steps in the code's words. Do not weaken the rule, and do not change the model to make it pass.
- If one step is impossible in the code (an action the model allows that a lock, a check or an ordering in the code prevents), it is a modeling error. Fix the model action so that it matches the code, and say which line of code the old action got wrong.

## Scope, bounds and assumptions

What is checked: two windows (processes) on the same book with one shared data folder, three commits (saves or whole-book reverts) in total, and one outside write.
- `checks/crashes`: up to two process crashes and one outside write, at any step, with no power cuts. About 4.2 M states.
- `checks/power-cut`: one power cut and one outside write, with no process crashes. About 2.2 M states.
- `checks/crash-then-power-cut`: one process crash, then one power cut. About 0.5 M states.
- `checks/failed-flushes`: any book-folder fsync may fail, plus one crash and one power cut. About 1.0 M states.

Each check covers everything within its bounds and nothing beyond them. A one-off run of `failed-flushes` with four commits and one outside write (66 M states, about 3 minutes) also passed on 2026-11-10.

The model assumes:
- `rename` is atomic, and renames in one folder reach the disk in order. A power cut leaves the book as any version renamed since the folder's last successful fsync (`bookCands`), and a file whose data was never fsynced comes back torn.
- Ledger writes (`Slot::store`) and backup creation always succeed and are durable. Their own failure paths are not modeled.
- Book-folder fsync (`sync_dir` in `Book::commit` and `Book::recover_locked`) succeeds, except in `checks/failed-flushes`, where any of them may fail. A `sync_dir` that returns Ok is trusted. `sync_dir` maps `InvalidInput` and `Unsupported` to Ok, and that is outside the model.
- Every outside write is durable. It may change the `FileStamp` (inode, length or mtime) or keep it, as an in-place edit of the same length that keeps the mtime does. The model tracks which writes kept it in `quiet`, and `StampSame(p)` is the code's stamp comparison.
- The commit lock (`BookData::lock`) is exclusive while held and released on crash. A window that cannot get it gives up, as the code does after 100 tries.

Left out: chapter-level revert, which is a normal rewrite commit and is covered by "save"; deleted-chapter records; history snapshots; aliases (`record_alias`); drafts; the content of a rewrite, which `verify_rewrite` and the proptest and fuzz suites in `crates/core/src/proofs.rs` cover; Windows; and more than two windows or three commits.

## Findings

Fixed on 2026-11-10. Each has a regression test in `crates/core/src/commit/tests.rs` that simulates the power cut with the crash seam (`Durability`: a power cut keeps any version renamed since the last book-folder sync), and a mutant here that undoes the fix.

1. **Recovery settled a rename that was never flushed.** After a crash between the rename and the folder fsync, `BookData::recover` on reopen set `head` to the new version without fsyncing the book's folder, so a power cut could leave the old file under a ledger naming the new one. `Book::recover_locked` now fsyncs the book's folder first (`Step::RecoveryDirSynced`). Test: `a_power_cut_after_a_crash_and_reopen_keeps_revert_to_the_original`. Mutant: `recover_without_sync`.
2. **Commit settled after a failed folder flush.** `Book::commit` now leaves the ledger pending when `sync_dir` fails, and `recover` resolves it by the file's real hash. Test: `a_power_cut_after_a_failed_folder_flush_keeps_revert_to_the_original`. Mutant: `settle_despite_failed_flush`.
3. **A later save reused a slot with an unresolved pending.** `Book::commit` now drops a cached slot whose in-memory ledger has a pending head, so `ensure_slot` runs recovery (folder fsync, then resolve) before writing a new pending. Test: `a_power_cut_after_two_failed_folder_flushes_keeps_revert_to_the_original`. Mutant: `keep_pending_slot`.
4. **`find_valid` hid an unresolved pending.** It used to clear `pending_head` in the window's in-memory copy, so a window that skipped recovery at open (the lock was busy) and then showed backup status or reverted wrote a ledger that dropped the old head. It now keeps the disk's pending. Test: `a_power_cut_after_a_crash_another_window_did_not_recover_keeps_revert_to_the_original`. Mutant: `find_valid_clears_pending`.
5. **A stale cached slot survived another window's write.** When the stamp changed but the bytes still matched the cached head (window A saved, then reverted), `Book::check_unchanged` kept window B's cached ledger, and B's next pending write erased A's head. It now drops the cached slot in that case. Test: `a_power_cut_after_another_window_reverted_keeps_revert_to_the_original`. Mutant: `keep_stale_slot`.

6. **A same-size outside edit that kept its mtime was overwritten.** Fixed on 2026-11-22; its test needs no power cut. The stamp checks saw no change, so the rename replaced the outside edit. `Book::open_with` now records the book's content hash, `Book::commit` compares the file's hash with it just before the stamp recheck, and the first save's backup is checked against that same hash. Test: `a_same_size_outside_edit_that_keeps_its_mtime_is_not_overwritten`. Mutant: `stamp_only_recheck`.

Still open, kept as `findings/external-write-between-recheck-and-rename` (must keep reproducing), which breaks `NoExternalWriteOverwritten`: another program writes the book after `Book::commit` rechecks the stamp (`Step::StampRechecked`) but before `fs::rename`, and the rename replaces that write without a warning. The window is a few microseconds and needs a compare-and-swap rename to close. It is a known limit. The narrower rule `NoMissedExternalWrite`, which covers any change before the recheck, holds.

## Coverage

The model was last confirmed against the code at **commit `ba47b58`** (2026-11-22). `coverage.json` stores a hash of each covered function body as it was at that commit. `run.sh drift` names each function whose text has changed since.

| Model action | Code |
|---|---|
| `OpenBook`, `OpenRecover` | `Book::open_with`, which reads the `FileStamp` and the content hash (`WriterState::hash`), then calls `recover` when `BookData::has_any_slot` (lock tried, error ignored); `recover_locked` fsyncs the book's folder, then `BookData::recover` |
| `StartSave`, `StartRevert`, `TakeLock` | `Book::save_changes`, `save_chapter`, `manage` and `revert` taking `lock_state` and `BookData::lock` |
| `CheckUnchanged` | `Book::check_unchanged`: the same stamp (no hash), or a different stamp but the on-disk hash equals the cached `slot.ledger.head`, in which case the cached slot is dropped and `state.hash` takes the on-disk hash |
| `ShowBackupStatus` | `Book::backup_status` → `valid_slot` → `BookData::find_valid`, which caches a slot whose in-memory ledger keeps the disk's `pending_head` |
| `RevertSlot` | `Book::revert(WholeBook)` → `valid_slot` → `find_valid` (no recover), plus the `at_base` early return, then into `commit` |
| `EnsureSlot`, `RecoverThenFind` | the cached-slot drop at the top of `Book::commit` when `pending_head` is set (or when `Slot::is_stored` finds its ledger gone, which needs another window deleting backups and is outside the model), then `Book::ensure_slot(state.hash)` → `recover_locked` (folder fsync, may fail) → `BookData::ensure` → `find_valid`, or `create` when the path still hashes to `state.hash` (`copy_verified`) |
| `CreateBackup`, `CreateLedger` | `BookData::create`: copy, fsync and rename `book.epub` and sync the folders, then `Slot::store(Created)` |
| `WriteTemp`, `SyncTemp` | `create_temp`, `rewrite` or `Payload::Restore` into the temp, then `sync_file(.., TempSynced)` |
| `SetPending` | `slot.set_pending(hash)`, which writes the window's in-memory ledger with `pending_head` |
| `RecheckStamp` | the on-disk content hash compared with `state.hash` as the save began, then the `FileStamp::of(&on_disk) != stamp` check, then `slot.clear_pending()` on failure of either |
| `RenameOverBook` | `fs::rename(&temp.path, &self.path)` |
| `SyncBookDir` | `sync_dir(dir)` after `Step::BookRenamed`; on failure, `commit` skips the settle |
| `Settle` | `slot.settle(hash)`, only after a successful folder fsync |
| `Finish` | `state.stamp = new_stamp` and the head reader replaced |
| `Crash`, `PowerCut` | a process dying at any step; power loss that keeps fsynced data and any version renamed since the last folder fsync |
| `ExternalWrite` | another program replacing the book file, or editing it in place while keeping its `FileStamp` |

The covered functions are listed in `scripts/coverage.py` (`COVERED`, 30 functions; `coverage.py list` prints them).

## Does my change touch this?

Run `run.sh drift`. If it reports none, the model still describes the code, so run `run.sh checks` if you changed nearby behaviour, and you are done. A change to any of these also counts even when drift misses it: the order of `hit(Step::…)` steps in `Book::commit`, how `Slot::store` writes, `FileStamp` fields, or a new caller that writes the book path without going through `Book::commit`. The last of these is a new action the model lacks.

## When the covered code changed

1. Run `git diff <the commit named under Coverage> -- crates/core/src/commit.rs crates/core/src/backup.rs crates/core/src/commit/seam.rs` and read only the functions that drift named.
2. For each changed function, find its actions in the Coverage table and decide: does the model still do exactly what the code now does, step for step, with the same indivisible steps?
3. If yes, the change is invisible at this level (renaming, logging, error text). Run `run.sh` and go to step 6.
4. If no, edit the matching action in `model/BookCommit.tla` to mirror the new code. Do not touch the invariants or the mutant flags. Run `run.sh`. Then:
   - `checks` FAIL: read the trace (see Reading a result). If the change was meant to fix one of the Findings, its config should now say `CHANGED`. Move it to `checks/`.
   - A mutant MISSED: your edit probably bypassed the step the mutant breaks. Make the mutant's `Mutant = "…"` branch still skip that step.
5. A new step with a durability effect (a new fsync, rename or ledger write) gets its own action, a row in the Coverage table and, if it guards a rule, a new mutant config under `mutants/` that removes it and must be `CAUGHT`.
6. Only after `run.sh` says `run: as expected` and you have checked the model against the new code, run `python3 .claude/skills/verify-book-commit/scripts/coverage.py stamp`. Then update the commit and date in the Coverage section to the new HEAD. Never stamp just to silence drift.

The repo rule of no comments in any file applies here too. The mapping lives in this file, not in the spec.
