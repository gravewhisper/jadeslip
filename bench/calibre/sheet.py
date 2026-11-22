import json
import subprocess
import sys
from pathlib import Path

runs = Path(sys.argv[1])
want = sys.argv[2]
out = Path(sys.argv[3])
for line in open(runs / "runs.jsonl"):
    r = json.loads(line)
    if r["run_id"] != want:
        continue
    d = runs / "frames" / r["run_id"]
    args = []
    for f in r["frames"]:
        if f["file"]:
            png = d / f["file"].replace(".ppm", ".png")
            args += ["-label", f"{f['file'][:-4]} {round(f['end_ms'] - r['spawn_ms'])} ms", str(png)]
    args += ["-label", "final", str(d / "final.png")]
    subprocess.run(["magick", "montage", *args, "-tile", "4x", "-geometry", "480x300+3+3", "-pointsize", "18", str(out)], check=True)
