import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

SKILL = Path(__file__).resolve().parent.parent
ROOT = SKILL.parent.parent.parent
RECORD = SKILL / "coverage.json"

COVERED = [
    ("crates/core/src/commit.rs", "FileStamp", "of"),
    ("crates/core/src/commit.rs", "Book", "open_with"),
    ("crates/core/src/commit.rs", "Book", "changed_on_disk"),
    ("crates/core/src/commit.rs", "Book", "recover"),
    ("crates/core/src/commit.rs", "Book", "recover_locked"),
    ("crates/core/src/commit.rs", "Book", "check_unchanged"),
    ("crates/core/src/commit.rs", "Book", "backup_status"),
    ("crates/core/src/commit.rs", "Book", "ensure_slot"),
    ("crates/core/src/commit.rs", "Book", "valid_slot"),
    ("crates/core/src/commit.rs", "Book", "revert"),
    ("crates/core/src/commit.rs", "Book", "commit"),
    ("crates/core/src/commit.rs", "", "remove_stale_temps"),
    ("crates/core/src/commit.rs", "", "create_temp"),
    ("crates/core/src/backup.rs", "BackupLedger", "matches"),
    ("crates/core/src/backup.rs", "Slot", "open_verified"),
    ("crates/core/src/backup.rs", "Slot", "is_stored"),
    ("crates/core/src/backup.rs", "Slot", "store"),
    ("crates/core/src/backup.rs", "Slot", "set_pending"),
    ("crates/core/src/backup.rs", "Slot", "clear_pending"),
    ("crates/core/src/backup.rs", "Slot", "settle"),
    ("crates/core/src/backup.rs", "BookData", "lock"),
    ("crates/core/src/backup.rs", "BookData", "slots"),
    ("crates/core/src/backup.rs", "BookData", "has_any_slot"),
    ("crates/core/src/backup.rs", "BookData", "recover"),
    ("crates/core/src/backup.rs", "BookData", "find_valid"),
    ("crates/core/src/backup.rs", "BookData", "ensure"),
    ("crates/core/src/backup.rs", "BookData", "create"),
    ("crates/core/src/backup.rs", "BookData", "copy_verified"),
    ("crates/core/src/commit/seam.rs", "", "sync_file"),
    ("crates/core/src/commit/seam.rs", "", "sync_dir"),
]


def block_end(text, start):
    depth = 0
    at = text.index("{", start)
    while at < len(text):
        ch = text[at]
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                return at + 1
        at += 1
    return len(text)


def owner_spans(text, owner):
    if not owner:
        return [(0, len(text))]
    spans = []
    for found in re.finditer(r"^impl(?:<[^>]*>)?\s+" + re.escape(owner) + r"\b[^{]*\{", text, re.M):
        spans.append((found.start(), block_end(text, found.start())))
    return spans


def bodies(path, owner, name):
    text = (ROOT / path).read_text(encoding="utf-8")
    found = []
    for start, end in owner_spans(text, owner):
        for hit in re.finditer(r"\bfn\s+" + re.escape(name) + r"\b", text[start:end]):
            at = start + hit.start()
            if not owner and any(s <= at < e for s, e in impl_spans(text)):
                continue
            found.append(text[at:block_end(text, at)])
    return found


def impl_spans(text):
    return [(m.start(), block_end(text, m.start())) for m in re.finditer(r"^impl\b[^{]*\{", text, re.M)]


def fingerprint(path, owner, name):
    found = bodies(path, owner, name)
    if not found:
        return None
    return hashlib.sha256("\n".join(found).encode()).hexdigest()


def label(path, owner, name):
    return f"{path}  {owner + '::' if owner else ''}{name}"


def head():
    return subprocess.run(["git", "-C", str(ROOT), "rev-parse", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()


def check():
    if not RECORD.exists():
        print("drift: no coverage.json; run coverage.py stamp after confirming the model")
        return 1
    record = json.loads(RECORD.read_text())
    known = {(item["file"], item["owner"], item["fn"]): item["sha256"] for item in record["items"]}
    changed = []
    for key in COVERED:
        now = fingerprint(*key)
        if now is None:
            changed.append(("MISSING", key))
        elif key not in known:
            changed.append(("NEW", key))
        elif known[key] != now:
            changed.append(("CHANGED", key))
    print(f"drift: model last confirmed against {record['sync_commit'][:12]}, HEAD is {head()[:12]}")
    if not changed:
        print(f"drift: none, all {len(COVERED)} covered functions match the confirmed text")
        return 0
    for kind, key in changed:
        print(f"drift: {kind:8} {label(*key)}")
    print("drift: the model may no longer describe the code; read SKILL.md 'When the covered code changed'")
    return 1


def stamp():
    items = []
    for path, owner, name in COVERED:
        value = fingerprint(path, owner, name)
        if value is None:
            sys.exit(f"stamp: {label(path, owner, name)} not found; fix COVERED first")
        items.append({"file": path, "owner": owner, "fn": name, "sha256": value})
    RECORD.write_text(json.dumps({"sync_commit": head(), "items": items}, indent=1) + "\n")
    print(f"stamp: recorded {len(items)} functions at {head()[:12]}")
    return 0


def main(argv):
    command = argv[0] if argv else "check"
    if command == "check":
        return check()
    if command == "stamp":
        return stamp()
    if command == "list":
        for key in COVERED:
            print(label(*key))
        return 0
    sys.exit("usage: coverage.py check|stamp|list")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
