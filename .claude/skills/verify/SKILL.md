---
name: verify
description: Run Jadeslip's standard verification (cargo test, clippy for both builds, the no-comments check, the attribution check, headless e2e suites, mutation runners, test leftovers) through scripts/verify.sh. Use before reporting a unit done, when accepting a unit as lead, or after touching an e2e suite.
---

# Verify

Every unit is checked with one script, run from the repo root or from any worktree of it:

```
scripts/verify.sh quick                       after each change, before you report
scripts/verify.sh full                        before a unit is accepted
scripts/verify.sh e2e reader edit             the GUI suites your change touches
scripts/verify.sh e2e edit -- a_saved_place   one suite, filtered by test name
scripts/verify.sh mutate chapters [filter]    a mutation runner, by name
scripts/verify.sh mutate search --changed     only the entries this branch touched
scripts/verify.sh mutate search --changed --list
                                              print those entries, run none
scripts/verify.sh leftovers                   only the leftover checks
scripts/verify.sh --base <ref> quick          attribution range other than merge-base with main
```

`mutate --changed` keeps the entries whose source file or test file differs between the merge base with main and the working tree, uncommitted and untracked files included. With nothing touched it selects no entry and passes. `--list` works with or without it, and so does a name filter. A runner takes the same flags directly: `python3 tests/mutate_search.py --changed --list`. Each case runs in its own session, and whatever is still in that session when the case ends is stopped.

`quick` runs the fixtures step, `cargo test --workspace`, clippy with `-D warnings` for the plain and the `--features jadeslip/e2e` build, `tests/check_no_comments.py`, and the attribution check over `base..HEAD`. `full` adds every GUI suite in `tests/e2e/tests`, one run of `tests/e2e/run.sh` each, then the leftover checks.

## Reading the output

The script ends with one table: step, PASS, FAIL, SKIP or BUSY, seconds, and a detail such as `374 passed, 0 failed, 30 ignored`. Exit 1 means at least one FAIL. Each step's full output is in the `logs:` folder printed under the table (`target/verify/<time>-<mode>/<step>.log`). Read the log before you guess at a cause.

- BUSY on `leftover-output` means another headless run holds the lock, and the run ends `verify: INCONCLUSIVE` with exit 3. It is not a pass. Wait and run `scripts/verify.sh leftovers` again.
- `leftover-folders` fails only on `jadeslip-test-*` folders whose owning process is dead. Folders of live processes are counted, not failed.
- An e2e row that says `nothing ran` means your filter matched no test.
- Report the table as it printed, with the commit it names. A self-report without it does not count.

## Worktrees

The tests find fixtures only at `<checkout>/fixtures`, compiled in through `CARGO_MANIFEST_DIR`. In a worktree with no `fixtures/real`, the fixtures step copies the real books from the main checkout, or from `SS_FIXTURES=<abs path to a fixtures folder>`, and generates `fixtures/generated` in place. It never symlinks, so a branch that regenerates fixtures cannot rewrite the main checkout's copy.

## Hard rules

- GUI tests run only through `tests/e2e/run.sh` or `tests/e2e/headless.sh`. Never start the app or a GUI test any other way. Test windows never go on the user's screen.
- Never dispatch focus, workspace or cursor moves, and never run `hyprctl reload`. Read-only `hyprctl ... -j` queries are fine.
- Mutation runners edit source files in place and restore them on exit. Run one alone in its checkout, never beside another verify run there. Stop one with Ctrl-C or SIGTERM, never SIGKILL. If the `mutate-*` row says the source was left mutated, restore it with git before anything else.
- No code comments in any file, tests and scripts included. `quick` enforces it.
- No AI attribution in commits. The check greps messages for `co-authored-by`, `claude` and `generated with`, so do not name the skills folder by path in a commit message either.
- Do not delete or edit `fixtures/real`. Copy a book before any write.
