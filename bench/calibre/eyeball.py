import json
import subprocess
import sys
from pathlib import Path

root = Path(sys.argv[1])
out = Path(sys.argv[2])
ink = {r["run_id"]: r for r in json.loads((root / "ink.json").read_text())}
args = []
for want in sys.argv[3:]:
    r = next(json.loads(l) for l in open(root / "runs.jsonl") if json.loads(l)["run_id"] == want)
    pick = ink[want]["first_frame"]
    kept = [f for f in r["frames"] if f["file"]]
    i = next(k for k, f in enumerate(kept) if f["file"] == pick)
    d = root / "frames" / want
    for f in kept[max(i - 1, 0):i + 1]:
        args += ["-label", f"{want[:3]} {want.split('-', 1)[1]} {f['file'][:-4]} {round(f['end_ms'] - r['spawn_ms'])}ms", str(d / f["file"].replace(".ppm", ".png"))]
    args += ["-label", f"{want[:3]} final", str(d / "final.png")]
subprocess.run(["magick", "montage", *args, "-tile", "3x", "-geometry", "560x350+3+3", "-pointsize", "16", str(out)], check=True)
