import json
import statistics
import sys
from collections import defaultdict
from pathlib import Path


def load(root):
    runs = [json.loads(line) for line in open(root / "runs.jsonl")]
    ink_file = root / "ink.json"
    ink = {row["run_id"]: row for row in json.loads(ink_file.read_text())} if ink_file.exists() else {}
    return runs, ink


def stat(values, unit="", digits=0):
    values = sorted(v for v in values if v is not None)
    if not values:
        return "n/a"
    med = statistics.median(values)
    fmt = f"{{:.{digits}f}}"
    return f"{fmt.format(med)}{unit} [{fmt.format(values[0])} to {fmt.format(values[-1])}] n={len(values)}"


def pss_at(run, at_ms):
    mem = run.get("mem") or []
    after = [m for m in mem if m["at_ms"] >= at_ms]
    pick = after[0] if after else (mem[-1] if mem else None)
    return pick["pss_kb"] / 1024 if pick else None


def compare(roots):
    runs, ink = [], {}
    for root in roots:
        rs, ik = load(root)
        for r in rs:
            r["run_id"] = f"{root.name}/{r['run_id']}"
        runs += rs
        ink.update({f"{root.name}/{k}": v for k, v in ik.items()})
    cells = defaultdict(list)
    for r in runs:
        if r["step"]["variant"] == "warmup":
            continue
        key = (r["step"]["book_label"], r["step"]["app"], r["step"]["variant"])
        cells[key].append(r)
    print("| book | app | variant | first text ms | fully drawn ms | SS internal mark ms | PSS MB at first text | PSS MB peak | PSS MB at end | end s after spawn | processes |")
    print("|---|---|---|---|---|---|---|---|---|---|---|")
    for key in sorted(cells):
        rs = cells[key]
        ext, internal, pss_first, pss_end, rss_end, procs = [], [], [], [], [], set()
        settled, peak, end_s = [], [], []
        for r in rs:
            row = ink.get(r["run_id"], {})
            first_ms = row.get("first_ms")
            ext.append(first_ms)
            if r.get("first_text_mark_ms"):
                internal.append(r["first_text_mark_ms"] - r["spawn_ms"])
            if first_ms is not None:
                pss_first.append(pss_at(r, r["spawn_ms"] + first_ms))
            a = r.get("analysis") or {}
            settled.append(a["first_end_ms"] - r["spawn_ms"] if a.get("first_end_ms") else None)
            mem = r.get("mem") or []
            peak.append(max(m["pss_kb"] for m in mem) / 1024 if mem else None)
            end_s.append((mem[-1]["at_ms"] - r["spawn_ms"]) / 1000 if mem else None)
            end = r.get("mem_end") or {}
            pss_end.append(end.get("pss_kb", 0) / 1024 or None)
            rss_end.append(end.get("rss_kb", 0) / 1024 or None)
            procs.add(end.get("processes"))
        missing = sum(1 for v in ext if v is None)
        ext_text = stat(ext) + (f", {missing} with no text" if missing else "")
        print(f"| {key[0]} | {key[1]} | {key[2]} | {ext_text} | {stat(settled)} | {stat(internal)} | {stat(pss_first)} | {stat(peak)} | {stat(pss_end)} | {stat(end_s, digits=1)} | {sorted(p for p in procs if p is not None)} |")


PHASES = ["main", "before_builder", "book_open", "start_chapter", "toc_parsed_to_start", "first_render", "setup",
          "window_built", "page_time_origin", "dom_content_loaded", "load_event_end", "first_text_called", "first_text_mark", "toc_called"]


def phase_row(r):
    sp = r["spawn_ms"]
    out = {name: at - sp for name, at in r.get("phases", [])}
    nav = r.get("nav") or {}
    origin = nav.get("timeOrigin")
    if origin:
        out["page_time_origin"] = origin - sp
        n = nav.get("nav") or {}
        if n.get("domContentLoadedEventEnd"):
            out["dom_content_loaded"] = origin + n["domContentLoadedEventEnd"] - sp
        if n.get("loadEventEnd"):
            out["load_event_end"] = origin + n["loadEventEnd"] - sp
    if r.get("first_text_mark_ms"):
        out["first_text_mark"] = r["first_text_mark_ms"] - sp
    return out


def builds(root):
    runs, _ = load(root)
    cells = defaultdict(list)
    for r in runs:
        if r["step"]["variant"] == "warmup":
            continue
        cells[(r["step"]["book_label"], r["step"]["variant"])].append(r)
    print("| book | build | first text mark ms (spawn to mark) | PSS MB at mark |")
    print("|---|---|---|---|")
    for key in sorted(cells):
        rs = cells[key]
        print(f"| {key[0]} | {key[1]} | {stat([r['first_text_mark_ms'] - r['spawn_ms'] for r in rs])} | {stat([r['mem_at_mark']['pss_kb'] / 1024 for r in rs])} |")
    for book in sorted({k[0] for k in cells}):
        variants = [k[1] for k in sorted(cells) if k[0] == book]
        print(f"\nPhases on {book}, median ms after spawn [range]:\n")
        print("| phase | " + " | ".join(variants) + " |")
        print("|---|" + "---|" * len(variants))
        rows = {v: [phase_row(r) for r in cells[(book, v)]] for v in variants}
        for name in PHASES:
            line = []
            for v in variants:
                values = [row[name] for row in rows[v] if name in row]
                line.append(stat(values, digits=1) if values else "absent")
            print(f"| {name} | " + " | ".join(line) + " |")


if __name__ == "__main__":
    roots = [Path(a) for a in sys.argv[2:]]
    compare(roots) if sys.argv[1] == "compare" else builds(roots[0])
