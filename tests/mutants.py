import re
import signal
import subprocess
import sys
import tempfile
import time
from functools import cache
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GRACE = 5

signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))


def run(command):
    if isinstance(command, str):
        command = ["sh", "-c", command]
    with tempfile.TemporaryFile() as log:
        case = subprocess.Popen(
            command,
            cwd=ROOT,
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        try:
            case.wait()
        finally:
            end_session(case)
        log.seek(0)
        return case.returncode == 0, log.read().decode(errors="replace")


NOT_RUN = (
    "error[",
    "could not compile",
    "GUI tests only run through tests/e2e/headless.sh",
    "no test window appeared",
    "the test window opened on",
    "the control socket never opened",
    "headless: ",
)


def not_run(output):
    for line in output.splitlines():
        if any(marker in line for marker in NOT_RUN):
            return line.strip()
    if "test result:" in output and not re.search(r"^running [1-9]", output, re.M):
        return "no test matched the filter"
    return None


def end_session(case):
    session = str(case.pid)
    subprocess.run(["pkill", "-TERM", "-s", session])
    deadline = time.monotonic() + GRACE
    while case.poll() is None or subprocess.run(["pgrep", "-s", session], stdout=subprocess.DEVNULL).returncode == 0:
        if time.monotonic() > deadline:
            subprocess.run(["pkill", "-KILL", "-s", session])
            break
        time.sleep(0.05)
    case.wait()


def test_file(crate, binary):
    return f"tests/e2e/tests/{binary}.rs" if crate == "e2e" else f"crates/{crate}/tests/{binary}.rs"


def node_tests():
    return re.findall(r"tests/ui/[\w.-]+\.mjs", (ROOT / "tests/e2e/tests/ui_node.rs").read_text())


@cache
def lib_tests(crate, test):
    src = ROOT / "crates" / crate / "src"
    found = []
    for path in sorted(src.rglob("*.rs")):
        text = path.read_text()
        if "#[test]" not in text:
            continue
        parts = path.relative_to(src).with_suffix("").parts
        module = "::".join(part for part in parts if part not in ("lib", "mod"))
        inner = [""] + [f"{name}::" for name in re.findall(r"\bmod (\w+) \{", text)]
        names = re.findall(r"\bfn (\w+)", text)
        if any(test in f"{module}::{mod}{name}".lstrip(":") for mod in inner for name in names):
            found.append(str(path.relative_to(ROOT)))
    return found


def changed_files():
    def git(*args):
        return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True, check=True).stdout

    try:
        base = git("merge-base", "HEAD", "main").strip()
        return set((git("diff", "--name-only", base) + git("ls-files", "--others", "--exclude-standard")).split())
    except subprocess.CalledProcessError as error:
        print(f"--changed: git failed: {error.stderr.strip()}")
        sys.exit(2)


def chosen(mutations, paths):
    flags = [arg for arg in sys.argv[1:] if arg.startswith("--")]
    words = [arg for arg in sys.argv[1:] if not arg.startswith("--")]
    if len(words) > 1 or set(flags) - {"--changed", "--list"}:
        print(f"usage: {Path(sys.argv[0]).name} [<filter>] [--changed] [--list]")
        sys.exit(2)
    wanted = words[0] if words else ""
    picked = [mutation for mutation in mutations if wanted in mutation[0]]
    if "--changed" in flags:
        touched = changed_files()
        picked = [mutation for mutation in picked if touched & set(paths(mutation))]
    if "--list" in flags or not picked:
        for mutation in picked:
            print(f"{mutation[0]}: {' '.join(dict.fromkeys(paths(mutation)))}")
        print(f"{len(picked)} of {len(mutations)} entries selected, none run")
        sys.exit(0)
    return picked
