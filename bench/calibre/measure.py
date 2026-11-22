import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUTPUT = "SS-TEST"
MARKER = "SS_SPEED_RUN"
CALIBRE_RULE_ON = (
    "SS_CALIBRE_RULE = SS_CALIBRE_RULE or hl.window_rule({ name = 'ss-calibre-silent', "
    "match = { class = '.*([Cc]alibre|ebook|libprs).*' }, workspace = '99 silent', no_initial_focus = true, "
    "suppress_event = 'fullscreen maximize activate activatefocus', no_anim = true }) "
    "SS_CALIBRE_RULE:set_enabled(true)"
)
CALIBRE_RULE_OFF = "if SS_CALIBRE_RULE then SS_CALIBRE_RULE:set_enabled(false) end"
SAMPLE_STEP = 97


def wall_ms():
    return time.time() * 1e3


class Abort(Exception):
    pass


def hypr_eval(script):
    reply = subprocess.run(["hyprctl", "eval", script], capture_output=True, text=True).stdout.strip()
    if reply != "ok":
        raise Abort(f"hyprctl eval failed: {reply}")


def hypr_json(*args):
    out = subprocess.run(["hyprctl", *args, "-j"], capture_output=True, text=True).stdout
    try:
        return json.loads(out)
    except json.JSONDecodeError:
        return []


def test_monitor_id():
    for monitor in hypr_json("monitors", "all"):
        if monitor["name"] == OUTPUT:
            return monitor["id"]
    raise Abort("the test output is missing")


def proc_table():
    table = {}
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            stat = Path(f"/proc/{entry}/stat").read_text()
        except OSError:
            continue
        tail = stat.rsplit(")", 1)[1].split()
        table[int(entry)] = (int(tail[1]), int(tail[3]), int(tail[11]) + int(tail[12]))
    return table


def descendants(root, table):
    tree = [root]
    at = 0
    while at < len(tree):
        parent = tree[at]
        tree.extend(pid for pid, row in table.items() if row[0] == parent)
        at += 1
    return [pid for pid in tree if pid in table]


def session_pids(sid, table):
    return [pid for pid, row in table.items() if row[1] == sid]


def kb(text, key):
    for line in text.splitlines():
        if line.startswith(key):
            return int(line.split()[1])
    return 0


def memory_of_tree(root):
    table = proc_table()
    pids = descendants(root, table)
    pss = rss = count = 0
    for pid in pids:
        try:
            rollup = Path(f"/proc/{pid}/smaps_rollup").read_text()
        except OSError:
            continue
        pss += kb(rollup, "Pss:")
        rss += kb(rollup, "Rss:")
        count += 1
    cpu = sum(table[pid][2] for pid in pids if pid in table)
    return {"pss_kb": pss, "rss_kb": rss, "processes": count, "cpu_ticks": cpu}


def marked_pids():
    found = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            env = Path(f"/proc/{entry}/environ").read_bytes()
        except OSError:
            continue
        if MARKER.encode() + b"=1" in env:
            found.append(int(entry))
    return found


def calibre_like(text):
    text = text.lower()
    return "calibre" in text or "ebook" in text or "libprs" in text


def calibre_processes():
    found = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            argv = Path(f"/proc/{entry}/cmdline").read_bytes().split(b"\0")[:2]
        except OSError:
            continue
        names = [os.path.basename(a.decode(errors="replace")) for a in argv]
        if any(n.startswith(("ebook-", "calibre")) for n in names):
            found.append((int(entry), names))
    return found


def calibre_windows():
    return [c for c in hypr_json("clients") if calibre_like(c.get("class", "") + c.get("initialClass", ""))]


def kill_session(sid):
    for sig in (signal.SIGTERM, signal.SIGKILL):
        table = proc_table()
        pids = session_pids(sid, table)
        if not pids:
            break
        for pid in pids:
            try:
                os.kill(pid, sig)
            except ProcessLookupError:
                pass
        deadline = time.time() + 3
        while time.time() < deadline and session_pids(sid, proc_table()):
            time.sleep(0.05)


def kill_marked():
    for _ in range(3):
        pids = marked_pids()
        if not pids:
            return
        for pid in pids:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        time.sleep(0.3)


def sampled_diff(a, b):
    sa = a[::SAMPLE_STEP]
    sb = b[::SAMPLE_STEP]
    if len(sa) != len(sb):
        return 1.0
    x = (int.from_bytes(sa, "little") ^ int.from_bytes(sb, "little")).to_bytes(len(sa), "little")
    return 1.0 - x.count(0) / len(sa)


def grab():
    return subprocess.run(["grim", "-o", OUTPUT, "-t", "ppm", "-"], capture_output=True).stdout


class Recorder:
    def __init__(self, run_dir, root_pid, sid, monitor_id, keep_threshold, fast_s):
        self.run_dir = run_dir
        self.root = root_pid
        self.sid = sid
        self.monitor_id = monitor_id
        self.keep_threshold = keep_threshold
        self.fast_s = fast_s
        self.stop = threading.Event()
        self.frames = []
        self.mem = []
        self.windows = []
        self.abort = None
        self.last_change_ms = None
        self.started_ms = wall_ms()

    def capture(self):
        kept = None
        kept_at = 0.0
        latest = None
        n = 0
        while not self.stop.is_set():
            begin = wall_ms()
            data = grab()
            end = wall_ms()
            if not data:
                continue
            latest = data
            if kept is None:
                change = 1.0
            elif data == kept:
                change = 0.0
            else:
                change = sampled_diff(data, kept)
            keep = kept is None or change > self.keep_threshold or (change > 0 and end - kept_at > 1000)
            entry = {"n": n, "begin_ms": begin, "end_ms": end, "change_vs_kept": change, "file": None}
            if change > self.keep_threshold:
                self.last_change_ms = end
            if keep:
                name = f"f{n:05d}.ppm"
                (self.run_dir / name).write_bytes(data)
                entry["file"] = name
                kept = data
                kept_at = end
            self.frames.append(entry)
            n += 1
            if end - self.started_ms > self.fast_s * 1000:
                time.sleep(0.05)
        if latest is not None:
            (self.run_dir / "final.ppm").write_bytes(latest)

    def watch(self):
        last_mem = 0.0
        while not self.stop.is_set():
            at = wall_ms()
            if at - last_mem >= 250:
                sample = memory_of_tree(self.root)
                sample["at_ms"] = at
                self.mem.append(sample)
                last_mem = at
            table = proc_table()
            ours = set(session_pids(self.sid, table))
            for client in hypr_json("clients"):
                klass = client.get("class", "") + "|" + client.get("initialClass", "")
                interesting = client.get("pid") in ours or calibre_like(klass) or "spinesprint" in klass.lower()
                if not interesting:
                    continue
                seen = {"at_ms": at, "class": client.get("class"), "initialClass": client.get("initialClass"),
                        "monitor": client.get("monitor"), "workspace": client.get("workspace", {}).get("name"),
                        "pid": client.get("pid"), "title_len": len(client.get("title", ""))}
                if not any(w["class"] == seen["class"] and w["monitor"] == seen["monitor"] for w in self.windows):
                    self.windows.append(seen)
                if client.get("monitor") not in (self.monitor_id, -1):
                    kill_session(self.sid)
                    self.abort = f"window {klass} opened on monitor {client.get('monitor')}, not {OUTPUT}"
                    self.stop.set()
                    return
            self.stop.wait(0.05 if at - self.started_ms < 10000 else 0.25)

    def start(self):
        self.threads = [threading.Thread(target=self.capture, daemon=True), threading.Thread(target=self.watch, daemon=True)]
        for t in self.threads:
            t.start()

    def finish(self):
        self.stop.set()
        for t in self.threads:
            t.join(timeout=10)


def make_env(extra):
    env = dict(os.environ)
    env[MARKER] = "1"
    env.update(extra)
    return env


def ctl(sock_file, command):
    sock_file.write((json.dumps(command) + "\n").encode())
    sock_file.flush()
    line = sock_file.readline()
    return json.loads(line) if line else None


def connect(path, deadline):
    while True:
        try:
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            s.connect(str(path))
            return s
        except OSError:
            s.close()
            if time.time() > deadline:
                raise Abort("control socket never opened")
            time.sleep(0.002)


NAV_JS = "(()=>{const n=performance.getEntriesByType('navigation')[0];const marks=performance.getEntriesByType('mark').map(m=>[m.name,m.startTime]);return {timeOrigin:performance.timeOrigin,nav:n?n.toJSON():null,marks};})()"
FIRST_TEXT_JS = "(async()=>{for(;;){const mark=performance.getEntriesByName('ss-first-text')[0];if(mark)return mark.detail;await new Promise((done)=>setTimeout(done,5))}})()"
TEXT_LEN_JS = "(()=>{const h=document.getElementById('book-host');return h&&h.shadowRoot?h.shadowRoot.textContent.length:-1})()"


def run_spinesprint(run_dir, binary, book, capture, tail_s=1.0):
    work = run_dir / "work"
    work.mkdir(parents=True)
    sock = Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp")) / f"ss-speed-{os.getpid()}-{run_dir.name[:3]}.sock"
    sock.unlink(missing_ok=True)
    (work / "tmp").mkdir()
    env = make_env({"TMPDIR": str(work / "tmp"), "SPINESPRINT_E2E_SOCKET": str(sock),
                    "SPINESPRINT_DATA_DIR": str(work / "data"), "SPINESPRINT_E2E_OFF": ""})
    log = open(run_dir / "app.stderr.log", "wb")
    monitor_id = test_monitor_id()
    spawn_ms = wall_ms()
    child = subprocess.Popen([str(binary), str(book)], env=env, stdout=subprocess.DEVNULL, stderr=log, start_new_session=True)
    rec = Recorder(run_dir, child.pid, child.pid, monitor_id, 0.002, 30) if capture else None
    if rec:
        rec.started_ms = spawn_ms
        rec.start()
    result = {"spawn_ms": spawn_ms, "pid": child.pid}
    try:
        s = connect(sock, time.time() + 20)
        s.settimeout(90)
        f = s.makefile("rwb")
        deadline = time.time() + 60
        while True:
            state = ctl(f, {"op": "state"})
            if any(p[0] == "first_text_called" for p in (state or {}).get("phases", [])):
                break
            if time.time() > deadline:
                raise Abort("page never asked for first text")
            time.sleep(0.01)
        reply = ctl(f, {"op": "eval", "js": FIRST_TEXT_JS, "timeout": 60000})
        if not reply or not reply.get("ok"):
            raise Abort(f"first text never appeared: {reply}")
        result["first_text_mark_ms"] = reply["v"]
        result["mem_at_mark"] = memory_of_tree(child.pid)
        result["mem_at_mark"]["at_ms"] = wall_ms()
        if rec:
            time.sleep(tail_s)
        result["phases"] = ctl(f, {"op": "state"}).get("phases", [])
        result["nav"] = ctl(f, {"op": "eval", "js": NAV_JS, "timeout": 5000}).get("v")
        result["text_len"] = ctl(f, {"op": "eval", "js": TEXT_LEN_JS, "timeout": 5000}).get("v")
        result["ink"] = ctl(f, {"op": "snapshot"}).get("ink")
        result["mem_end"] = memory_of_tree(child.pid)
        clients = [c for c in hypr_json("clients") if c.get("pid") == child.pid]
        result["window"] = [{"class": c.get("class"), "monitor": c.get("monitor")} for c in clients]
        if not clients or any(c.get("monitor") != monitor_id for c in clients):
            raise Abort(f"spinesprint window not on {OUTPUT}: {result['window']}")
        f.close()
        s.close()
    finally:
        if rec:
            rec.finish()
            result.update(frames=rec.frames, mem=rec.mem, windows=rec.windows)
            if rec.abort:
                result["abort"] = rec.abort
        kill_session(child.pid)
        child.wait()
        log.close()
        shutil.rmtree(work, ignore_errors=True)
        sock.unlink(missing_ok=True)
    if rec and rec.abort:
        raise Abort(rec.abort)
    return result


def run_calibre(run_dir, tool, book, profile_dir, cap_s, quiet_s, extra_args=()):
    profile_dir.mkdir(parents=True, exist_ok=True)
    for sub in ("config", "cache", "tmp"):
        (profile_dir / sub).mkdir(exist_ok=True)
    seed = profile_dir / "config" / "viewer-webengine.json"
    if tool == "ebook-viewer" and not seed.exists():
        seed.write_text(json.dumps({"old_prefs_migrated": True, "session_data": {"controls_help_shown_count": 2}}))
    env = make_env({"CALIBRE_CONFIG_DIRECTORY": str(profile_dir / "config"),
                    "CALIBRE_CACHE_DIRECTORY": str(profile_dir / "cache"),
                    "CALIBRE_TEMP_DIR": str(profile_dir / "tmp"), "TMPDIR": str(profile_dir / "tmp")})
    for leftover in marked_pids():
        raise Abort(f"a marked process is still alive before launch: {leftover}")
    if calibre_windows():
        raise Abort("a calibre window exists before launch")
    running = calibre_processes()
    if running:
        raise Abort(f"a calibre process is running before launch: {running}")
    monitor_id = test_monitor_id()
    log = open(run_dir / "app.stderr.log", "wb")
    args = [tool, str(book), *extra_args]
    if tool == "ebook-viewer":
        args.insert(1, "--new-instance")
    spawn_ms = wall_ms()
    child = subprocess.Popen(args, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    rec = Recorder(run_dir, child.pid, child.pid, monitor_id, 0.002, 8)
    rec.started_ms = spawn_ms
    rec.start()
    result = {"spawn_ms": spawn_ms, "pid": child.pid, "tool": tool}
    try:
        while True:
            time.sleep(0.1)
            now = wall_ms()
            if rec.abort:
                break
            if now - spawn_ms > cap_s * 1000:
                result["capped"] = True
                break
            if child.poll() is not None:
                result["exited"] = child.returncode
                time.sleep(0.5)
                break
            have_window = any(w for w in rec.windows)
            last = rec.last_change_ms or spawn_ms
            cpu_quiet = False
            if len(rec.mem) >= 9:
                window = rec.mem[-9:]
                span = (window[-1]["at_ms"] - window[0]["at_ms"]) / 1000
                ticks = window[-1]["cpu_ticks"] - window[0]["cpu_ticks"]
                cpu_quiet = span > 0 and ticks / 100 / span < 0.05
            if have_window and now - last > quiet_s * 1000 and cpu_quiet:
                break
    finally:
        rec.finish()
        result["mem_end"] = memory_of_tree(child.pid)
        kill_session(child.pid)
        kill_marked()
        child.wait()
        log.close()
        result.update(frames=rec.frames, mem=rec.mem, windows=rec.windows)
        if rec.abort:
            result["abort"] = rec.abort
        deadline = time.time() + 5
        while time.time() < deadline and (calibre_windows() or marked_pids()):
            time.sleep(0.1)
        result["leftover_windows"] = len(calibre_windows())
        result["leftover_pids"] = marked_pids()
    if rec.abort:
        raise Abort(rec.abort)
    if result["leftover_windows"] or result["leftover_pids"]:
        raise Abort(f"calibre left windows or processes: {result['leftover_windows']} {result['leftover_pids']}")
    return result


def to_png(run_dir):
    ppms = sorted(run_dir.glob("*.ppm"))
    if not ppms:
        return
    subprocess.run(["magick", "mogrify", "-format", "png", "-define", "png:compression-level=3", *map(str, ppms)], check=True)
    for p in ppms:
        p.unlink()


def first_text_analysis(run_dir, frames, threshold):
    final_ppm = run_dir / "final.ppm"
    if not final_ppm.exists():
        return {}
    final = final_ppm.read_bytes()
    kept = [f for f in frames if f["file"]]
    diffs = []
    for f in kept:
        data = (run_dir / f["file"]).read_bytes()
        d = sampled_diff(data, final)
        f["diff_vs_final"] = d
        diffs.append(d)
    first = None
    for i, f in enumerate(kept):
        if all(d <= threshold for d in diffs[i:]):
            first = f
            break
    out = {"threshold": threshold, "kept_frames": len(kept), "total_frames": len(frames)}
    if first:
        before = [f for f in frames if f["n"] < first["n"]]
        out.update(first_frame=first["file"], first_begin_ms=first["begin_ms"], first_end_ms=first["end_ms"],
                   prev_frame_end_ms=before[-1]["end_ms"] if before else None)
    return out


def main():
    plan = json.loads(Path(sys.argv[1]).read_text())
    out_dir = HERE / plan["out"]
    out_dir.mkdir(parents=True, exist_ok=True)
    results = out_dir / "runs.jsonl"
    uses_calibre = any(step["app"] in ("ebook-viewer", "ebook-edit") for step in plan["steps"])

    def on_signal(signum, frame):
        raise SystemExit(128 + signum)

    for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(sig, on_signal)

    guard = None
    try:
        time.sleep(plan.get("settle_s", 3))
        if uses_calibre:
            guard = subprocess.Popen(["setsid", "bash", str(HERE / "rule_guard.sh"), str(os.getpid())],
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            hypr_eval(CALIBRE_RULE_ON)
        with open(results, "a") as sink:
            for index, step in enumerate(plan["steps"]):
                run_id = f"{index:03d}-{step['app']}-{step['book_label']}-{step.get('variant', '')}"
                run_dir = out_dir / "frames" / run_id
                run_dir.mkdir(parents=True, exist_ok=True)
                load = Path("/proc/loadavg").read_text().split()[:3]
                print(f"{time.strftime('%H:%M:%S')} {run_id} load {' '.join(load)}", flush=True)
                if step["app"] == "spinesprint":
                    r = run_spinesprint(run_dir, HERE / step["binary"], HERE / step["book"], step.get("capture", True))
                else:
                    cap = step.get("cap_s", 300)
                    r = run_calibre(run_dir, step["app"], HERE / step["book"], HERE / step["profile"], cap, step.get("quiet_s", 5), step.get("args", []))
                    if step.get("drop_profile"):
                        shutil.rmtree(step["profile"], ignore_errors=True)
                r.update(run_id=run_id, step=step, loadavg_before=load)
                if step.get("capture", True):
                    r["analysis"] = first_text_analysis(run_dir, r["frames"], plan.get("threshold", 0.01))
                    to_png(run_dir)
                sink.write(json.dumps(r) + "\n")
                sink.flush()
                summary = {k: r.get(k) for k in ("first_text_mark_ms", "capped", "exited")}
                a = r.get("analysis", {})
                if a.get("first_end_ms"):
                    summary["external_ms"] = round(a["first_end_ms"] - r["spawn_ms"])
                if r.get("first_text_mark_ms"):
                    summary["internal_ms"] = round(r["first_text_mark_ms"] - r["spawn_ms"])
                print("   ", json.dumps(summary), flush=True)
                time.sleep(step.get("pause_s", 0.3))
    finally:
        kill_marked()
        if uses_calibre:
            try:
                hypr_eval(CALIBRE_RULE_OFF)
            except Abort as error:
                print(error, file=sys.stderr)
        if guard:
            guard.kill()


if __name__ == "__main__":
    main()
