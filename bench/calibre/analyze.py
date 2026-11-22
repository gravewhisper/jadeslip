import json
import subprocess
import sys
from collections import Counter
from pathlib import Path

REGIONS = {
    "spinesprint": (240, 70, 1030, 680),
    "ebook-viewer": (20, 40, 1240, 740),
    "ebook-edit": (300, 80, 960, 640),
    "ebook-edit-open": (10, 80, 280, 640),
    "ebook-edit-pane": (300, 165, 320, 430),
}
PANE_DELTA = 60
PANE_SHARE = 0.01
INK_DELTA = 60
INK_SHARE = 0.25


def region_of(run):
    app = run["step"]["app"]
    if app == "ebook-edit" and not run["step"].get("args"):
        return REGIONS["ebook-edit-open"]
    return REGIONS[app]


def ink(png, box):
    w, h, x, y = box[2], box[3], box[0], box[1]
    raw = subprocess.run(["magick", str(png), "-crop", f"{w}x{h}+{x}+{y}", "+repage", "-colorspace", "gray",
                          "-sample", "50%", "gray:-"], capture_output=True, check=True).stdout
    background = Counter(raw).most_common(1)[0][0]
    return sum(1 for v in raw if abs(v - background) > INK_DELTA) / len(raw)


def gray(png, box):
    w, h, x, y = box[2], box[3], box[0], box[1]
    return subprocess.run(["magick", str(png), "-crop", f"{w}x{h}+{x}+{y}", "+repage", "-colorspace", "gray",
                           "gray:-"], capture_output=True, check=True).stdout


def pane_settled(run, d):
    box = REGIONS["ebook-edit-pane"]
    final = gray(d / "final.png", box)
    series = []
    for f in run["frames"]:
        if f["file"] and f["n"] > 0:
            raw = gray(d / f["file"].replace(".ppm", ".png"), box)
            off = sum(1 for a, b in zip(raw, final) if abs(a - b) > PANE_DELTA) / len(final)
            series.append((f, off))
    for i, (f, off) in enumerate(series):
        if all(o <= PANE_SHARE for _, o in series[i:]):
            prev_end = None
            for g in run["frames"]:
                if g["n"] < f["n"]:
                    prev_end = g["end_ms"]
            return {"final_ink": ink(d / "final.png", box), "detector": "pane-settled",
                    "series": [(g["file"], round(g["end_ms"] - run["spawn_ms"]), round(o, 4)) for g, o in series],
                    "first_frame": f["file"], "first_ms": f["end_ms"] - run["spawn_ms"],
                    "prev_frame_ms": (prev_end - run["spawn_ms"]) if prev_end else None, "first_ink": None}
    return {"final_ink": 0, "detector": "pane-settled", "series": []}


def analyze(run, frames_root):
    d = frames_root / run["run_id"]
    if run["step"]["app"] == "ebook-edit" and run["step"].get("args"):
        return pane_settled(run, d)
    box = region_of(run)
    final_ink = ink(d / "final.png", box)
    series = []
    for f in run["frames"]:
        if f["file"] and f["n"] > 0:
            series.append((f, ink(d / f["file"].replace(".ppm", ".png"), box)))
    first = None
    for i, (f, value) in enumerate(series):
        if value >= INK_SHARE * final_ink and final_ink > 0:
            first = (i, f, value)
            break
    out = {"detector": "ink", "final_ink": final_ink, "series": [(f["file"], round(f["end_ms"] - run["spawn_ms"]), round(v, 4)) for f, v in series]}
    if first:
        i, f, value = first
        prev_end = None
        for g in run["frames"]:
            if g["n"] < f["n"]:
                prev_end = g["end_ms"]
        out.update(first_frame=f["file"], first_ms=f["end_ms"] - run["spawn_ms"],
                   prev_frame_ms=(prev_end - run["spawn_ms"]) if prev_end else None, first_ink=value)
    return out


def main():
    root = Path(sys.argv[1])
    rows = []
    for line in open(root / "runs.jsonl"):
        run = json.loads(line)
        if not run["step"].get("capture", True):
            continue
        result = analyze(run, root / "frames")
        rows.append({"run_id": run["run_id"], **{k: v for k, v in result.items() if k != "series"}})
        if "-v" in sys.argv:
            print(run["run_id"], json.dumps(result))
        else:
            print(run["run_id"], result.get("first_frame"), round(result.get("first_ms") or -1), round(result["final_ink"], 4))
    (root / "ink.json").write_text(json.dumps(rows, indent=1))


if __name__ == "__main__":
    main()
