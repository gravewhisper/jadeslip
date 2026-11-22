import json
import os
import shutil
import socket
import sys
import time
from pathlib import Path

SKILL = Path(__file__).resolve().parent.parent
RUNS = Path(os.environ.get("JSV_RUNS", SKILL.parent.parent.parent / "target" / "verify-app"))

KEYS = {
    "Enter": 0xFF0D,
    "Escape": 0xFF1B,
    "Tab": 0xFF09,
    "Backspace": 0xFF08,
    "Delete": 0xFFFF,
    "Left": 0xFF51,
    "Up": 0xFF52,
    "Right": 0xFF53,
    "Down": 0xFF54,
    "PageUp": 0xFF55,
    "PageDown": 0xFF56,
    "Home": 0xFF50,
    "End": 0xFF57,
    "F11": 0xFFC8,
    "Space": 0x20,
}
MASKS = {"shift": 1, "ctrl": 4, "alt": 8}

USAGE = """usage: ctl.sh <command> [args]
  ready [seconds]            wait until the page is usable (book text shown, or the library drawn)
  state                      the app's own state op: url, windows, fullscreen, zoom, phases
  eval JS [ms]               run JS in the page and print its JSON value
  wait JS [seconds]          poll JS until it returns a truthy value
  find SEL                   print the box of the first visible match; SEL is CSS, text=Label, or book:<either> inside the book
  click SEL | click-at X Y   real GDK click at the element's centre
  right-click SEL            real GDK right click
  dblclick SEL               real GDK double click
  type TEXT                  real key events, one per character, into the focused element
  key NAME [ctrl] [shift]    Enter Escape Tab Backspace Delete Left Up Right Down PageUp PageDown Home End F11 Space, or one character
  shot NAME                  PNG of the visible window into evidence/NAME.png
  keep PATH [NAME]           copy a file or folder from the run (data/..., books/...) into evidence/
  drop PATH...               hand files to the window as a drag and drop would
  save-answer PATH|cancel    answer the next native save dialog (test seam, say so in the proof)
  note TEXT                  add a line to evidence/journal.log
  quit                       ask the app to exit
  run                        print the run folder"""


def run_dir():
    named = os.environ.get("JSV_RUN")
    path = Path(named) if named else RUNS / "current"
    if not path.exists():
        sys.exit(f"ctl: no run at {path}; start one with launch.sh")
    return path.resolve()


def journal(run, line):
    evidence = run / "evidence"
    evidence.mkdir(exist_ok=True)
    stamp = time.strftime("%H:%M:%S")
    with open(evidence / "journal.log", "a", encoding="utf-8") as out:
        out.write(f"{stamp} {line}\n")


class Control:
    def __init__(self, run, connect_seconds=5.0):
        self.run = run
        path = os.path.realpath(run / "ctl.sock")
        deadline = time.monotonic() + connect_seconds
        while True:
            try:
                self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                self.sock.connect(path)
                break
            except OSError as error:
                self.sock.close()
                if time.monotonic() > deadline:
                    sys.exit(f"ctl: the control socket {path} does not answer ({error}); run doctor.sh")
                time.sleep(0.05)
        self.sock.settimeout(120)
        self.reader = self.sock.makefile("r", encoding="utf-8")

    def op(self, command):
        self.sock.sendall((json.dumps(command) + "\n").encode())
        line = self.reader.readline()
        if not line:
            sys.exit("ctl: the app closed the control socket")
        return json.loads(line)

    def js(self, script, timeout_ms=30000):
        reply = self.op({"op": "eval", "js": script, "timeout": timeout_ms})
        if not reply.get("ok"):
            raise RuntimeError(reply.get("err", "no reply"))
        return reply.get("v")


FIND = r"""(() => {
  const wanted = %s;
  const shown = (e) => { const r = e.getBoundingClientRect(); const s = getComputedStyle(e);
    return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && !e.closest('[hidden]'); };
  let all;
  let scope = document;
  let query = wanted;
  if (query.startsWith('book:')) { scope = document.getElementById('book-host').shadowRoot; query = query.slice(5); }
  if (query.startsWith('text=')) {
    const label = query.slice(5).trim();
    all = [...scope.querySelectorAll('button,a,[role],label,li,.card,.row,.result,input,summary,p,h1,h2,h3,h4,span')]
      .filter((e) => (e.textContent || '').trim() === label || e.getAttribute('aria-label') === label);
  } else {
    all = [...scope.querySelectorAll(query)];
  }
  const e = all.find(shown);
  if (!e) return { found: false, matches: all.length };
  const r = e.getBoundingClientRect();
  const x = r.left + r.width / 2, y = r.top + r.height / 2;
  const onTop = scope.elementFromPoint(x, y);
  const inView = x >= 0 && y >= 0 && x <= innerWidth && y <= innerHeight;
  const covered = onTop && !(e === onTop || e.contains(onTop) || onTop.contains(e));
  return { found: true, x, y, w: r.width, h: r.height, inView, coveredBy: covered ? (onTop.id || onTop.className || onTop.tagName) : null,
           tag: e.tagName, id: e.id, text: (e.textContent || '').trim().slice(0, 80) };
})()"""


def find(control, selector):
    box = control.js(FIND % json.dumps(selector))
    if not box or not box.get("found"):
        sys.exit(f"ctl: no visible element for {selector!r} ({(box or {}).get('matches', 0)} hidden matches)")
    if not box["inView"]:
        sys.exit(f"ctl: {selector!r} is outside the window at {box['x']:.0f},{box['y']:.0f}; scroll it into view the way a user would")
    if box["coveredBy"]:
        sys.exit(f"ctl: {selector!r} is covered by {box['coveredBy']!r}; close that first")
    return box


def keyval(name):
    if name in KEYS:
        return KEYS[name]
    if len(name) == 1:
        point = ord(name)
        return point if point < 0x100 else 0x01000000 | point
    sys.exit(f"ctl: unknown key {name!r}")


def print_json(value):
    print(json.dumps(value, indent=2, ensure_ascii=False))


READY_BOOK = "(() => !!performance.getEntriesByName('ss-first-text')[0] || !document.getElementById('open-failure').hidden)()"
READY_SHELF = "(() => !document.getElementById('library').hidden && document.readyState === 'complete')()"


def wait_for(control, script, seconds):
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        try:
            last = control.js(script, 5000)
        except RuntimeError as error:
            last = f"error: {error}"
        if last and not (isinstance(last, str) and last.startswith("error: ")):
            return last
        time.sleep(0.1)
    raise TimeoutError(f"still not true after {seconds}s (last value {last!r})")


def main(argv):
    if not argv or argv[0] in ("-h", "--help", "help"):
        print(USAGE)
        return 0
    command, args = argv[0], argv[1:]
    run = run_dir()
    if command == "run":
        print(run)
        return 0
    if command == "note":
        journal(run, "note " + " ".join(args))
        return 0
    if command == "keep":
        source = (run / args[0]).resolve()
        if not source.is_relative_to(run) or not source.exists():
            sys.exit(f"ctl: {args[0]} is not inside the run folder")
        target = run / "evidence" / (args[1] if len(args) > 1 else source.name)
        if source.is_dir():
            shutil.copytree(source, target, dirs_exist_ok=True)
        else:
            shutil.copy2(source, target)
        journal(run, f"keep {args[0]} -> {target.relative_to(run)}")
        print(target)
        return 0
    control = Control(run, 60 if command == "ready" else 5)
    if command == "ready":
        seconds = float(args[0]) if args else 60
        has_book = any((run / "books").iterdir()) if (run / "books").exists() else False
        try:
            wait_for(control, READY_BOOK if has_book else READY_SHELF, seconds)
        except TimeoutError as error:
            journal(run, f"ready FAILED {error}")
            sys.exit(f"ctl: not ready: {error}")
        failed = control.js("document.getElementById('open-failure').hidden ? '' : document.getElementById('open-failure-text').textContent")
        journal(run, "ready" + (f" (open failure: {failed})" if failed else ""))
        if failed:
            sys.exit(f"ctl: the book did not open: {failed}")
        print("ready")
        return 0
    if command == "state":
        reply = control.op({"op": "state"})
        journal(run, "state")
        print_json(reply)
        return 0 if reply.get("ok") else 1
    if command == "eval":
        try:
            value = control.js(args[0], int(args[1]) if len(args) > 1 else 30000)
        except RuntimeError as error:
            journal(run, f"eval FAILED {error}: {args[0][:200]}")
            sys.exit(f"ctl: page script failed: {error}")
        journal(run, f"eval {args[0][:200]} => {json.dumps(value, ensure_ascii=False)[:300]}")
        print_json(value)
        return 0
    if command == "wait":
        try:
            value = wait_for(control, args[0], float(args[1]) if len(args) > 1 else 15)
        except TimeoutError as error:
            journal(run, f"wait FAILED {error}: {args[0][:200]}")
            sys.exit(f"ctl: {error}")
        journal(run, f"wait {args[0][:200]} => {json.dumps(value, ensure_ascii=False)[:200]}")
        print_json(value)
        return 0
    if command == "find":
        print_json(find(control, args[0]))
        return 0
    if command in ("click", "right-click", "dblclick"):
        box = find(control, args[0])
        op = {"click": "click", "right-click": "right_click", "dblclick": "double_click"}[command]
        payload = {"op": op, "x": box["x"], "y": box["y"]}
        if op == "double_click":
            payload["gtk_sequence"] = True
        control.op(payload)
        journal(run, f"{command} {args[0]} at {box['x']:.0f},{box['y']:.0f} ({box['tag']}#{box['id']} {box['text'][:40]!r})")
        return 0
    if command == "click-at":
        control.op({"op": "click", "x": float(args[0]), "y": float(args[1])})
        journal(run, f"click-at {args[0]},{args[1]}")
        return 0
    if command == "type":
        for ch in args[0]:
            control.op({"op": "key", "keyval": keyval(ch)})
        journal(run, f"type {args[0]!r}")
        return 0
    if command == "key":
        state = sum(MASKS[mod] for mod in args[1:])
        control.op({"op": "key", "keyval": keyval(args[0]), "state": state})
        journal(run, f"key {' '.join(args)}")
        return 0
    if command == "shot":
        target = run / "evidence" / f"{args[0]}.png"
        target.parent.mkdir(exist_ok=True)
        reply = control.op({"op": "snapshot", "png": str(target)})
        if not reply.get("ok") or not target.exists():
            sys.exit(f"ctl: snapshot failed: {reply}")
        journal(run, f"shot {target.relative_to(run)} {reply['width']}x{reply['height']}")
        print(target)
        return 0
    if command == "drop":
        paths = [str(Path(path).resolve()) for path in args]
        control.op({"op": "drop_paths", "paths": paths})
        journal(run, f"drop {paths}")
        return 0
    if command == "save-answer":
        payload = {"op": "arm_save_answer"}
        if args and args[0] != "cancel":
            payload["path"] = str(Path(args[0]).resolve())
        control.op(payload)
        journal(run, f"save-answer {args[0] if args else 'cancel'} (test seam)")
        return 0
    if command == "quit":
        control.op({"op": "quit"})
        journal(run, "quit")
        return 0
    sys.exit(f"ctl: unknown command {command!r}\n{USAGE}")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
