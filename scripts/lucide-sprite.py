"""Rebuild the Lucide icon sprite inside app/ui/index.html.

usage: python3 -I scripts/lucide-sprite.py <lucide-static package folder>

Get the package with `npm pack lucide-static@0.460.0` and untar it. Every
`#i-<name>` used in app/ui/*.html or app/ui/*.js becomes one <symbol>, so the
sprite holds exactly the icons the UI references. The package LICENSE is copied
to app/ui/LICENSE-Lucide.txt.
"""

import json
import re
import shutil
import sys
from pathlib import Path

UI = Path(__file__).resolve().parent.parent / "app" / "ui"
SPRITE = re.compile(r'<svg class="sprite" id="icons" aria-hidden="true">.*?</svg>', re.S)
USE = re.compile(r"#i-([a-z0-9-]+)")
SHAPE = re.compile(r"<(path|circle|rect|line|polyline|polygon|ellipse)\b([^>]*?)\s*/>", re.S)


def used_names():
    names = set()
    for path in sorted(UI.glob("*.html")) + sorted(UI.glob("*.js")):
        text = SPRITE.sub("", path.read_text())
        names.update(USE.findall(text))
    return sorted(names)


def symbol(icons, name):
    source = (icons / f"{name}.svg").read_text()
    shapes = "".join(
        f"<{tag} {' '.join(attrs.split())}/>" for tag, attrs in SHAPE.findall(source)
    )
    if not shapes:
        sys.exit(f"lucide-sprite: no shapes in {name}.svg")
    return f'<symbol id="i-{name}" viewBox="0 0 24 24">{shapes}</symbol>'


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    package = Path(sys.argv[1])
    icons = package / "icons"
    version = json.loads((package / "package.json").read_text())["version"]
    names = used_names()
    missing = [name for name in names if not (icons / f"{name}.svg").is_file()]
    if missing:
        sys.exit(f"lucide-sprite: not in lucide-static {version}: {', '.join(missing)}")
    sprite = (
        '<svg class="sprite" id="icons" aria-hidden="true">'
        + "".join(symbol(icons, name) for name in names)
        + "</svg>"
    )
    index = UI / "index.html"
    html = index.read_text()
    if not SPRITE.search(html):
        sys.exit("lucide-sprite: index.html has no sprite element to replace")
    index.write_text(SPRITE.sub(lambda _: sprite, html, count=1))
    shutil.copyfile(package / "LICENSE", UI / "LICENSE-Lucide.txt")
    print(f"lucide-sprite: {len(names)} icons from lucide-static {version}")


if __name__ == "__main__":
    main()
