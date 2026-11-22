import json
import sys
from pathlib import Path

S = Path(__file__).resolve().parent
B = Path("books")
BOOKS = {
    "h1p": (B / "large-h1-p.epub", "OEBPS/Text/c00000.xhtml"),
    "real": (B / "real-serial_a.epub", "EPUB/volume_1.xhtml"),
    "divbr": (B / "large-h1-div-br.epub", "OEBPS/Text/c00000.xhtml"),
}
BIN = {name: str(Path("bin") / f"spinesprint-{name}-e2e") for name in ("head", "48183bb", "70bb680")}


def ss(label, build, capture=True):
    return {"app": "spinesprint", "binary": BIN[build], "book": str(BOOKS[label][0]), "book_label": label,
            "variant": build + ("" if capture else "-nocap"), "capture": capture}


def calibre_steps(label, tag):
    book, chapter = BOOKS[label]
    profiles = Path("profiles")
    return [
        {"app": "ebook-viewer", "book": str(book), "book_label": label, "variant": "cold", "profile": str(profiles / f"{tag}-viewer"), "cap_s": 300},
        {"app": "ebook-viewer", "book": str(book), "book_label": label, "variant": "warm", "profile": str(profiles / f"{tag}-viewer"), "cap_s": 300, "drop_profile": True},
        {"app": "ebook-edit", "book": str(book), "book_label": label, "variant": "open", "profile": str(profiles / f"{tag}-edit-open"), "cap_s": 120, "drop_profile": True},
        {"app": "ebook-edit", "book": str(book), "book_label": label, "variant": "chapter", "profile": str(profiles / f"{tag}-edit-ch"), "cap_s": 120, "args": [chapter], "drop_profile": True},
    ]


def comparison(rounds, start=0, out="compare"):
    steps = [dict(ss("h1p", "head"), variant="warmup")] + [dict(s, variant="warmup") for s in calibre_steps("h1p", "warmup")]
    for r in range(start, start + rounds):
        order = ["h1p", "real", "divbr"] if r % 2 == 0 else ["divbr", "real", "h1p"]
        for label in order:
            steps.append(ss(label, "head"))
            steps += calibre_steps(label, f"r{r}-{label}")
            steps.append(ss(label, "head"))
    return {"out": out, "steps": steps}


def builds(rounds):
    steps = [dict(ss("real", b, False), variant="warmup") for b in ("head", "48183bb", "70bb680")]
    for r in range(rounds):
        builds_order = ["head", "48183bb", "70bb680"] if r % 2 == 0 else ["70bb680", "48183bb", "head"]
        for label in ("real", "h1p"):
            for b in builds_order:
                steps.append(ss(label, b, False))
        steps.append(ss("real", "head", True))
        steps.append(ss("real", "70bb680", True))
    return {"out": "builds", "steps": steps}


if __name__ == "__main__":
    which, rounds = sys.argv[1], int(sys.argv[2])
    start = int(sys.argv[3]) if len(sys.argv) > 3 else 0
    out = sys.argv[4] if len(sys.argv) > 4 else which
    plan = comparison(rounds, start, out) if which == "compare" else builds(rounds)
    Path(S / f"plan-{out}.json").write_text(json.dumps(plan, indent=1))
    print(which, len(plan["steps"]), "steps")
