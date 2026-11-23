# Contributing

Bug reports and pull requests are welcome. For anything bigger than a small fix, open an issue first so we can agree on the approach.

## Running the tests

```sh
cargo run --release -p jadeslip-fixtures --bin gen-fixtures   # generate the test books once
cargo test --workspace     # unit and integration tests
scripts/verify.sh quick    # tests, clippy for both builds, and the repo's style checks
scripts/verify.sh full     # plus every real-app suite
scripts/screenshots.sh     # retake the README screenshots and demo GIF in the real app
```

`verify.sh` also runs the tests that need real books. Those aren't in the repo, so put a few EPUBs of your own in `fixtures/real/` first (the folder is gitignored). The real-app suites in `full` drive the actual window with real clicks and key presses on a hidden Hyprland output, so they need Hyprland.

## House rules

- No code comments. Names and tests carry the meaning.
- Design decisions and their reasons go in [`docs/DESIGN.md`](docs/DESIGN.md).
- A change to saving, backups or history must keep the TLA+ models passing. See [`.claude/skills/verify-book-commit/`](.claude/skills/verify-book-commit/SKILL.md) and [`.claude/skills/verify-book-history/`](.claude/skills/verify-book-history/SKILL.md).
