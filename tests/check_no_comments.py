import io
import subprocess
import sys
import tokenize
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SKIPPED_PREFIXES = ("docs/", "crates/core/tests/corpus/")


def line_of(text, at):
    return text.count("\n", 0, at) + 1


def skip_quoted(text, at, quote):
    at += 1
    while at < len(text):
        if text[at] == "\\":
            at += 2
        elif text[at] == quote:
            return at + 1
        else:
            at += 1
    return at


def raw_rust_string_end(text, at):
    hashes = 0
    probe = at + 1
    while probe < len(text) and text[probe] == "#":
        hashes += 1
        probe += 1
    if probe >= len(text) or text[probe] != '"':
        return None
    closing = '"' + "#" * hashes
    found = text.find(closing, probe + 1)
    return len(text) if found < 0 else found + len(closing)


def rust_comments(text):
    found = []
    at = 0
    while at < len(text):
        ch = text[at]
        previous = text[at - 1] if at else " "
        if text.startswith(("//", "/*"), at):
            found.append(line_of(text, at))
            end = text.find("\n" if text[at + 1] == "/" else "*/", at)
            at = len(text) if end < 0 else end + 1
        elif ch == "r" and not (previous.isalnum() or previous == "_"):
            end = raw_rust_string_end(text, at)
            at = end if end else at + 1
        elif ch == '"':
            at = skip_quoted(text, at, '"')
        elif ch == "'":
            literal = text[at + 1 : at + 3] == "\\" + text[at + 2 : at + 3] or text[at + 2 : at + 3] == "'"
            at = skip_quoted(text, at, "'") if literal else at + 1
        else:
            at += 1
    return found


def js_comments(text):
    found = []
    at = 0
    last_significant = ""
    while at < len(text):
        ch = text[at]
        if text.startswith(("//", "/*"), at):
            found.append(line_of(text, at))
            end = text.find("\n" if text[at + 1] == "/" else "*/", at)
            at = len(text) if end < 0 else end + 1
            continue
        if ch in "\"'`":
            at = skip_quoted(text, at, ch)
            last_significant = ch
            continue
        if ch == "/" and last_significant in ("", "(", ",", "=", ":", "[", "!", "&", "|", "?", "{", "}", ";"):
            at = skip_quoted(text, at, "/")
            last_significant = "/"
            continue
        if not ch.isspace():
            last_significant = ch
        at += 1
    return found


def css_comments(text):
    found = []
    at = 0
    while at < len(text):
        if text[at] in "\"'":
            at = skip_quoted(text, at, text[at])
        elif text.startswith("/*", at):
            found.append(line_of(text, at))
            at += 2
        else:
            at += 1
    return found


def html_comments(text):
    return [line_of(text, at) for at in range(len(text)) if text.startswith("<!--", at)]


def hash_comments(text):
    found = []
    for number, line in enumerate(text.splitlines(), 1):
        if number == 1 and line.startswith("#!"):
            continue
        quote = None
        at = 0
        while at < len(line):
            ch = line[at]
            if quote:
                if ch == "\\" and quote != "'":
                    at += 1
                elif ch == quote:
                    quote = None
            elif ch in "\"'":
                quote = ch
            elif ch == "#" and (at == 0 or line[at - 1].isspace()):
                found.append(number)
                break
            at += 1
    return found


def python_comments(text):
    tokens = tokenize.generate_tokens(io.StringIO(text).readline)
    return [token.start[0] for token in tokens if token.type == tokenize.COMMENT]


CHECKS = {
    ".rs": rust_comments,
    ".js": js_comments,
    ".css": css_comments,
    ".html": html_comments,
    ".sh": hash_comments,
    ".toml": hash_comments,
    ".py": python_comments,
}


def main():
    listed = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.splitlines()
    checked = {}
    failures = []
    for name in listed:
        path = ROOT / name
        check = CHECKS.get(path.suffix)
        if check is None or name.startswith(SKIPPED_PREFIXES) or not path.exists():
            continue
        checked[path.suffix] = checked.get(path.suffix, 0) + 1
        failures += [f"{name}:{line}" for line in check(path.read_text(encoding="utf-8"))]
    print("files checked:", dict(sorted(checked.items())))
    print("comments found:", len(failures))
    for failure in failures:
        print(" ", failure)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
