import html.parser
import json
import posixpath
import sys
import urllib.parse
import xml.etree.ElementTree as ET
import zipfile

OPF_NS = "{http://www.idpf.org/2007/opf}"
NCX_NS = "{http://www.daisy.org/z3986/2005/ncx/}"
XHTML_NS = "{http://www.w3.org/1999/xhtml}"
EPUB_TYPE = "{http://www.idpf.org/2007/ops}type"
CONTAINER_NS = "{urn:oasis:names:tc:opendocument:xmlns:container}"


def resolve(base, href):
    path = urllib.parse.unquote(href.split("#")[0].split("?")[0])
    return posixpath.normpath(posixpath.join(posixpath.dirname(base), path))


def is_remote(href):
    return "://" in href or href.startswith("mailto:") or href.startswith("data:")



def check_nav_list(nav_name, element, names, problems, what):
    items = element.findall(f"{XHTML_NS}li")
    if not items:
        problems.append(f"{what}: a list with no items")
    for item in items:
        anchor = item.find(f"{XHTML_NS}a")
        span = item.find(f"{XHTML_NS}span")
        nested = item.find(f"{XHTML_NS}ol")
        if nested is None:
            nested = item.find(f"{XHTML_NS}ul")
        if anchor is None and span is None:
            problems.append(f"{what}: an item with no label")
        if anchor is None and nested is None:
            problems.append(f"{what}: a span with no nested list")
        if anchor is not None:
            href = anchor.get("href")
            if href and not is_remote(href) and not href.startswith("#"):
                if resolve(nav_name, href) not in names:
                    problems.append(f"{what}: a link to a missing file")
                yield resolve(nav_name, href)
        if nested is not None:
            yield from check_nav_list(nav_name, nested, names, problems, what)


class LinkParser(html.parser.HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.hrefs = []

    def handle_starttag(self, tag, attrs):
        if tag.split(":")[-1] not in ("a", "area"):
            return
        for name, value in attrs:
            if name.split(":")[-1] == "href" and value:
                self.hrefs.append(value)


def decoded(raw):
    if raw[:2] in (b"\xff\xfe", b"\xfe\xff"):
        return raw.decode("utf-16", "replace")
    return raw.decode("utf-8", "replace")


def dangling_links(book, doc_name, names):
    parser = LinkParser()
    parser.feed(decoded(book.read(doc_name)))
    count = 0
    for href in parser.hrefs:
        if is_remote(href) or href.startswith("#") or ":" in href.split("/")[0]:
            continue
        if resolve(doc_name, href) not in names:
            count += 1
    return count


def walk_points(parent):
    for point in parent.findall(f"{NCX_NS}navPoint"):
        yield point
        yield from walk_points(point)


def in_spine_order(targets, spine_order):
    last = -1
    for target in targets:
        at = spine_order.get(target)
        if at is None:
            continue
        if at < last:
            return False
        last = at
    return True


def check(path):
    problems = []
    with zipfile.ZipFile(path) as book:
        infos = book.infolist()
        names = {info.filename for info in infos}
        if len(names) != len(infos):
            problems.append("duplicate zip names")
        if not infos or infos[0].filename != "mimetype":
            problems.append("mimetype is not the first record")
        elif infos[0].compress_type != zipfile.ZIP_STORED or infos[0].extra:
            problems.append("mimetype is not stored plainly")
        elif book.read("mimetype") != b"application/epub+zip":
            problems.append("mimetype has the wrong text")
        lower = [name.lower() for name in names]
        if len(set(lower)) != len(lower):
            problems.append("two zip names differ only by case")
        container = ET.fromstring(book.read("META-INF/container.xml"))
        rootfile = container.find(f".//{CONTAINER_NS}rootfile")
        opf_name = rootfile.get("full-path")
        opf = ET.fromstring(book.read(opf_name))
        manifest = opf.find(f"{OPF_NS}manifest")
        spine = opf.find(f"{OPF_NS}spine")
        guide = opf.find(f"{OPF_NS}guide")
        items = manifest.findall(f"{OPF_NS}item")
        rows = spine.findall(f"{OPF_NS}itemref")
        ids = {}
        hrefs = set()
        nav_items = []
        ncx_item = None
        for item in items:
            item_id = item.get("id")
            if item_id in ids:
                problems.append("duplicate manifest id")
            ids[item_id] = item
            href = item.get("href")
            if is_remote(href):
                continue
            target = resolve(opf_name, href)
            if target in hrefs:
                problems.append("duplicate manifest href")
            hrefs.add(target)
            if target not in names:
                problems.append("manifest item has no file")
            if "nav" in (item.get("properties") or "").split():
                nav_items.append(target)
            if item.get("media-type") == "application/x-dtbncx+xml":
                ncx_item = target
        all_ids = [node.get("id") for node in opf.iter() if node.get("id")]
        if len(all_ids) != len(set(all_ids)):
            problems.append("duplicate id in the package")
        if not rows:
            problems.append("empty spine")
        if not any(row.get("linear") != "no" for row in rows):
            problems.append("no linear spine row")
        seen = set()
        spine_order = {}
        for at, row in enumerate(rows):
            idref = row.get("idref")
            if idref in seen:
                problems.append("duplicate spine idref")
            seen.add(idref)
            if idref not in ids:
                problems.append("spine idref does not resolve")
            else:
                spine_order.setdefault(resolve(opf_name, ids[idref].get("href")), at)
        version = opf.get("version") or ""
        if version.startswith("3") and len(nav_items) != 1:
            problems.append("an EPUB3 book without exactly one nav item")
        if len(nav_items) > 1:
            problems.append("more than one nav item")
        if guide is not None:
            references = guide.findall(f"{OPF_NS}reference")
            if not references:
                problems.append("an empty guide")
            for reference in references:
                if not is_remote(reference.get("href")) and resolve(opf_name, reference.get("href")) not in names:
                    problems.append("a guide reference to a missing file")
        toc_attr = spine.get("toc")
        if toc_attr is not None and toc_attr not in ids:
            problems.append("spine toc does not resolve")
        summary = {"rows": len(rows), "items": len(items), "records": len(infos)}
        documents = sorted(
            resolve(opf_name, item.get("href"))
            for item in items
            if item.get("media-type") in ("application/xhtml+xml", "text/html") and not is_remote(item.get("href"))
        )
        dangling = sum(
            dangling_links(book, doc, names) for doc in documents if doc in names and doc not in nav_items
        )
        summary["dangling_links"] = dangling
        if dangling:
            problems.append("a chapter link to a missing file")
        for nav_name in nav_items:
            nav = ET.fromstring(book.read(nav_name))
            navs = list(nav.iter(f"{XHTML_NS}nav"))
            toc_navs = [node for node in navs if "toc" in (node.get(EPUB_TYPE) or "").split()]
            if len(toc_navs) > 1:
                problems.append("more than one toc nav")
            for node in navs:
                is_toc = node in toc_navs or not toc_navs
                listed = node.find(f"{XHTML_NS}ol")
                if listed is None:
                    listed = node.find(f"{XHTML_NS}ul")
                if listed is None:
                    problems.append("a nav with no list")
                    continue
                targets = list(check_nav_list(nav_name, listed, names, problems, "nav"))
                if is_toc:
                    summary["nav_links"] = len(targets)
                    summary["nav_ordered"] = in_spine_order(targets, spine_order)
        if ncx_item is not None:
            ncx = ET.fromstring(book.read(ncx_item))
            nav_map = ncx.find(f"{NCX_NS}navMap")
            points = list(walk_points(nav_map))
            if not points:
                problems.append("an empty navMap")
            point_ids = [point.get("id") for point in points if point.get("id")]
            if len(point_ids) != len(set(point_ids)):
                problems.append("duplicate navPoint id")
            targets = []
            last = 0
            for point in points:
                content = point.find(f"{NCX_NS}content")
                if content is None or not content.get("src"):
                    problems.append("a navPoint with no target")
                    continue
                target = resolve(ncx_item, content.get("src"))
                if target not in names:
                    problems.append("a navPoint target is missing")
                targets.append(target)
                label = point.find(f"{NCX_NS}navLabel/{NCX_NS}text")
                if label is None:
                    problems.append("a navPoint with no label")
                order = point.get("playOrder")
                if order is not None:
                    if not order.isdigit() or int(order) <= last:
                        problems.append("playOrder does not rise")
                    last = int(order) if order.isdigit() else last
            summary["ncx_links"] = len(targets)
            summary["ncx_ordered"] = in_spine_order(targets, spine_order)
    return problems, summary


def main():
    failed = False
    for path in sys.argv[1:]:
        try:
            problems, summary = check(path)
        except Exception as error:
            problems, summary = [f"unreadable: {type(error).__name__}"], {}
        print(json.dumps({"problems": sorted(set(problems)), **summary}))
        failed = failed or bool(problems)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
