import json
import re
import sys
import xml.etree.ElementTree as ET
import zipfile
from html.parser import HTMLParser
from posixpath import dirname, join, normpath
from urllib.parse import unquote

HIDDEN = {"script", "style", "template", "noscript", "head", "title", "iframe", "object", "embed", "noframes", "datalist", "select"}
VOID = {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"}
BLOCK = {
    "address", "article", "aside", "blockquote", "body", "center", "dd", "details", "dir", "div", "dl", "dt",
    "figcaption", "figure", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6", "header", "hr", "li", "main",
    "menu", "nav", "ol", "p", "pre", "section", "summary", "table", "tbody", "td", "tfoot", "th", "thead", "tr", "ul",
}
OPF = "{http://www.idpf.org/2007/opf}"


class Visible(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.parts = []
        self.hidden = 0
        self.in_body = False

    def handle_starttag(self, tag, attrs):
        tag = tag.lower()
        if tag == "body":
            self.in_body = True
        if tag in HIDDEN and tag not in VOID:
            self.hidden += 1
        if tag in BLOCK or tag == "br":
            self.parts.append("\n")

    def handle_startendtag(self, tag, attrs):
        if tag.lower() in BLOCK or tag.lower() == "br":
            self.parts.append("\n")

    def handle_endtag(self, tag):
        tag = tag.lower()
        if tag in HIDDEN and tag not in VOID and self.hidden > 0:
            self.hidden -= 1
        if tag in BLOCK:
            self.parts.append("\n")

    def handle_data(self, data):
        if self.in_body and self.hidden == 0:
            self.parts.append(data)


def spine_entries(archive):
    container = ET.fromstring(archive.read("META-INF/container.xml"))
    rootfile = next(node for node in container.iter() if node.tag.endswith("rootfile"))
    opf_name = rootfile.attrib["full-path"]
    opf = ET.fromstring(archive.read(opf_name))
    base = dirname(opf_name)
    items = {}
    for item in opf.find(f"{OPF}manifest"):
        items[item.attrib["id"]] = item.attrib
    entries = []
    for ref in opf.find(f"{OPF}spine"):
        item = items.get(ref.attrib["idref"])
        if item is None:
            continue
        if "nav" in item.get("properties", "").split():
            entries.append(None)
            continue
        entries.append(normpath(join(base, unquote(item["href"]))))
    return entries


def visible_text(raw):
    parser = Visible()
    parser.feed(raw.decode("utf-8", "replace"))
    parser.close()
    return "".join(parser.parts)


def main():
    path, *specs = sys.argv[1:]
    patterns = [(spec, re.compile(spec.split(":", 1)[1], re.I if spec.startswith("i:") else 0)) for spec in specs]
    with zipfile.ZipFile(path) as archive:
        names = set(archive.namelist())
        per_chapter = {spec: {} for spec, _ in patterns}
        chars = 0
        scanned = 0
        for ix, entry in enumerate(spine_entries(archive)):
            if entry is None or entry not in names:
                continue
            if not entry.lower().endswith((".xhtml", ".html", ".htm", ".xml")):
                continue
            text = visible_text(archive.read(entry))
            chars += len(text)
            scanned += 1
            for spec, pattern in patterns:
                found = sum(1 for _ in pattern.finditer(text))
                if found:
                    per_chapter[spec][str(ix)] = found
    json.dump({"scanned": scanned, "chars": chars, "counts": per_chapter}, sys.stdout)


if __name__ == "__main__":
    main()
