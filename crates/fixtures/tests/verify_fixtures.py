import json
import multiprocessing
import re
import struct
import sys
import xml.etree.ElementTree as ET
import xml.parsers.expat
import zipfile
import zlib
from html.parser import HTMLParser
from pathlib import Path

MIB = 1 << 20
BLOCK = {"p", "div", "h1", "h2", "h3", "ul", "ol", "li", "table", "tr", "td", "blockquote", "section", "pre"}
NON_RENDERED = {"script", "style", "head", "title", "template"}
WS = re.compile(r"[ \t\n\r\f]+")
PACKAGE_NS = "{http://www.idpf.org/2007/opf}"
XHTML_NS = "{http://www.w3.org/1999/xhtml}"
NCX_NS = "{http://www.daisy.org/z3986/2005/ncx/}"
LIMITS = {"max_entries": 200_000, "max_entry": 64 * MIB, "max_book": 4 << 30, "max_ratio": 1000, "max_depth": 128}


class Visible(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.in_body = False
        self.hidden = 0
        self.parts = []
        self.pending = ""

    def flush(self):
        if self.pending:
            self.parts.append(WS.sub(" ", self.pending))
            self.pending = ""

    def handle_starttag(self, tag, attrs):
        if tag == "body":
            self.in_body = True
        elif tag in NON_RENDERED:
            self.hidden += 1
        elif tag == "br" and self.visible():
            self.flush()
            self.parts.append("\n")
        elif tag in BLOCK and self.visible():
            self.flush()
            self.parts.append("\n")

    def handle_startendtag(self, tag, attrs):
        if tag == "br" and self.visible():
            self.flush()
            self.parts.append("\n")

    def handle_endtag(self, tag):
        if tag in NON_RENDERED:
            self.hidden = max(0, self.hidden - 1)
        elif tag in BLOCK and self.visible():
            self.flush()
            self.parts.append("\n")

    def handle_data(self, data):
        if self.visible():
            self.pending += data

    def visible(self):
        return self.in_body and self.hidden == 0

    def text(self):
        self.flush()
        return "".join(self.parts)


def decode(raw, encoding):
    if encoding.startswith("gbk"):
        return raw.decode("gb18030")
    return raw.decode("utf-8", errors="surrogateescape")


def is_well_formed(raw, encoding):
    parser = xml.parsers.expat.ParserCreate("utf-8" if encoding.startswith("gbk") else None)
    data = raw.decode("gb18030").encode("utf-8") if encoding.startswith("gbk") else raw
    try:
        parser.Parse(data, True)
        return True
    except (xml.parsers.expat.ExpatError, LookupError, ValueError):
        return False


def scan_chapter(job):
    path, name, encoding, terms, want_shape = job
    with zipfile.ZipFile(path) as archive:
        raw = archive.read(name)
    parser = Visible()
    parser.feed(decode(raw, encoding))
    parser.close()
    text = parser.text()
    lowered = text.lower()
    exact = {term: text.count(term) for term in terms}
    folded = {term: lowered.count(term.lower()) for term in terms}
    return {
        "name": name,
        "exact": exact,
        "folded": folded,
        "words": len(text.split()),
        "well_formed": is_well_formed(raw, encoding),
        "shape_ok": shape_ok(raw, encoding, want_shape),
    }


def shape_ok(raw, encoding, want):
    if want is None:
        return True
    text = decode(raw, encoding)
    body = text[text.index("<body>") + 6 : text.rindex("</body>")]
    if want == "h1,p":
        return re.fullmatch(r"\s*<h1>.*?</h1>\s*(<p>.*?</p>\s*)+", body, re.S) is not None and "<div" not in body
    return (
        re.fullmatch(r"\s*<h1>[^<]*</h1>\s*<div>.*</div>\s*", body, re.S) is not None
        and body.count("<div") == 1
        and body.count("<br/>") >= 20
        and "<p>" not in body
    )


def read_central(path):
    data = Path(path).read_bytes()
    eocd = data.rfind(b"PK\x05\x06")
    assert eocd >= 0, f"{path}: no end record"
    _, _, _, _, count, cd_len, cd_start, _ = struct.unpack("<IHHHHIIH", data[eocd : eocd + 22])
    if count == 0xFFFF:
        locator = eocd - 20
        assert data[locator : locator + 4] == b"PK\x06\x07", f"{path}: no zip64 locator"
        at = struct.unpack("<Q", data[locator + 8 : locator + 16])[0]
        assert data[at : at + 4] == b"PK\x06\x06", f"{path}: no zip64 end record"
        count, _, cd_len, cd_start = struct.unpack("<QQQQ", data[at + 24 : at + 56])
    rows = []
    at = cd_start
    for _ in range(count):
        assert data[at : at + 4] == b"PK\x01\x02", f"{path}: bad central record at {at}"
        method, crc, compressed, inflated, name_len, extra_len, comment_len = struct.unpack("<HxxxxIIIHHH", data[at + 10 : at + 34])
        offset = struct.unpack("<I", data[at + 42 : at + 46])[0]
        name = data[at + 46 : at + 46 + name_len]
        rows.append({"name": name, "method": method, "crc": crc, "compressed": compressed, "inflated": inflated, "offset": offset})
        at += 46 + name_len + extra_len + comment_len
    return data, rows


def local_body(data, row):
    at = row["offset"]
    assert data[at : at + 4] == b"PK\x03\x04", "bad local header"
    name_len, extra_len = struct.unpack("<HH", data[at + 26 : at + 30])
    start = at + 30 + name_len + extra_len
    return data[start : start + row["compressed"]], struct.unpack("<II", data[at + 18 : at + 26])


def inflate_row(data, row):
    body, _ = local_body(data, row)
    if row["method"] == 0:
        return body
    return zlib.decompressobj(-15).decompress(body)


def by_name(rows, name):
    return next(r for r in rows if r["name"] == name)


def check_bomb(truth, path):
    data, rows = read_central(path)
    book = truth["book"]
    named = {r["name"] for r in rows}
    bad_offsets = [r for r in rows if r["offset"] >= len(data)]
    if book == "bomb-ratio":
        row = by_name(rows, b"OEBPS/c0.xhtml")
        ratio = row["inflated"] / row["compressed"]
        assert ratio > LIMITS["max_ratio"], f"ratio {ratio}"
        assert MIB < row["inflated"] < LIMITS["max_entry"]
        assert len(inflate_row(data, row)) == row["inflated"]
        print(f"  {book}: ratio {ratio:.1f}:1, inflated {row['inflated']}")
    elif book == "bomb-oversize-entry":
        row = by_name(rows, b"OEBPS/c0.xhtml")
        ratio = row["inflated"] / row["compressed"]
        assert row["inflated"] > LIMITS["max_entry"] and ratio < LIMITS["max_ratio"], ratio
        assert len(inflate_row(data, row)) == row["inflated"]
        print(f"  {book}: ratio {ratio:.1f}:1, inflated {row['inflated']}")
    elif book == "bomb-book-total":
        chapters = [r for r in rows if r["name"].startswith(b"OEBPS/c") and r["name"].endswith(b".xhtml")]
        total = sum(r["inflated"] for r in chapters)
        assert total > LIMITS["max_book"]
        assert all(r["inflated"] < LIMITS["max_entry"] and r["inflated"] / r["compressed"] < LIMITS["max_ratio"] for r in chapters)
        assert len(inflate_row(data, chapters[-1])) == chapters[-1]["inflated"]
        print(f"  {book}: {len(chapters)} entries, {total} declared inflated bytes")
    elif book == "bomb-overlap-shared":
        chapters = [r for r in rows if r["name"].startswith(b"OEBPS/c") and r["name"].endswith(b".xhtml")]
        assert len(chapters) == 1000 and len({r["offset"] for r in chapters}) == 1
    elif book == "bomb-overlap-nested":
        outer, inner = by_name(rows, b"OEBPS/c0.xhtml"), by_name(rows, b"OEBPS/c1.xhtml")
        assert outer["offset"] < inner["offset"] < outer["offset"] + 30 + 14 + outer["compressed"]
        assert zlib.crc32(inflate_row(data, outer)) == outer["crc"]
        assert zlib.crc32(inflate_row(data, inner)) == inner["crc"]
        lower_bound_extent = 30 + len(outer["name"]) + outer["compressed"]
        assert lower_bound_extent > inner["offset"] - outer["offset"]
    elif book == "bomb-lying-sizes":
        rows_by = {i: by_name(rows, f"OEBPS/c{i}.xhtml".encode()) for i in range(5)}
        assert len(inflate_row(data, rows_by[0])) == rows_by[0]["inflated"]
        assert len(inflate_row(data, rows_by[1])) > 10 * MIB and rows_by[1]["inflated"] == 1024
        assert len(inflate_row(data, rows_by[2])) < 4096 and rows_by[2]["inflated"] == MIB
        _, local_sizes = local_body(data, rows_by[3])
        assert local_sizes == (1, 5) and rows_by[3]["compressed"] != 1 and len(inflate_row(data, rows_by[3])) == rows_by[3]["inflated"]
        assert zlib.crc32(inflate_row(data, rows_by[4])) != rows_by[4]["crc"]
    elif book == "bomb-lying-extent":
        assert by_name(rows, b"OEBPS/c1.xhtml")["offset"] + by_name(rows, b"OEBPS/c1.xhtml")["compressed"] > len(data)
        assert by_name(rows, b"OEBPS/c2.xhtml") in bad_offsets
    elif book == "bomb-huge-entry-count":
        assert len(rows) > LIMITS["max_entries"], len(rows)
        print(f"  {book}: {len(rows)} central records, {len(data)} bytes")
    elif book == "bomb-traversal-dotdot":
        assert any(b"/../" in n for n in named)
    elif book == "bomb-traversal-absolute":
        assert any(n.startswith(b"/") for n in named)
    elif book == "bomb-traversal-backslash":
        assert any(b"\\" in n for n in named)
    elif book == "bomb-traversal-nul":
        assert any(b"\0" in n for n in named)
    elif book == "bomb-traversal-opf-href":
        assert not any(b".." in n for n in named)
        assert b'href="../../../../etc/hostname"' in inflate_row(data, by_name(rows, b"OEBPS/content.opf"))
    elif book == "bomb-traversal-container":
        assert b'full-path="../../../etc/content.opf"' in inflate_row(data, by_name(rows, b"META-INF/container.xml"))
    elif book == "bomb-deep-xml-chapter":
        deep = inflate_row(data, by_name(rows, b"OEBPS/c1.xhtml"))
        assert deep.count(b"<div>") > LIMITS["max_depth"]
        assert is_well_formed(inflate_row(data, by_name(rows, b"OEBPS/c0.xhtml")), "utf-8")
    elif book == "bomb-deep-xml-opf":
        assert inflate_row(data, by_name(rows, b"OEBPS/content.opf")).count(b"<x>") > LIMITS["max_depth"]
    else:
        raise AssertionError(f"no bomb check for {book}")


def nav_rows(node, depth, out):
    for li in node.findall(f"{XHTML_NS}li"):
        link = li.find(f"{XHTML_NS}a")
        span = li.find(f"{XHTML_NS}span")
        if link is not None:
            out.append({"depth": depth, "label": "".join(link.itertext()), "target": link.get("href")})
        else:
            out.append({"depth": depth, "label": "".join(span.itertext()), "target": None})
        child = li.find(f"{XHTML_NS}ol")
        if child is not None:
            nav_rows(child, depth + 1, out)


def ncx_rows(node, depth, out):
    for point in node.findall(f"{NCX_NS}navPoint"):
        content = point.find(f"{NCX_NS}content")
        label = point.find(f"{NCX_NS}navLabel/{NCX_NS}text")
        out.append({"depth": depth, "label": label.text, "target": None if content is None else content.get("src")})
        ncx_rows(point, depth + 1, out)


def check_package(truth, archive):
    container = ET.fromstring(archive.read("META-INF/container.xml"))
    opf_path = container[0][0].get("full-path")
    if truth["book"] == "malformed-container-ref":
        assert opf_path not in archive.namelist()
        opf_path = "OEBPS/content.opf"
    opf = ET.fromstring(archive.read(opf_path))
    manifest = {i.get("id"): i for i in opf.find(f"{PACKAGE_NS}manifest")}
    spine = list(opf.find(f"{PACKAGE_NS}spine"))
    assert len(spine) == len(truth["spine"]), "spine length"
    assert opf.find(f"{PACKAGE_NS}metadata/{{http://purl.org/dc/elements/1.1/}}title").text == truth["title"]
    names = set(archive.namelist())
    linear = 0
    for ref, row in zip(spine, truth["spine"]):
        item = manifest.get(ref.get("idref"))
        assert row["linear"] == (ref.get("linear") != "no"), row
        if row["kind"] == "unresolved-idref":
            assert item is None and row["href"] is None
            continue
        assert item.get("href") == row["href"], row
        present = f"OEBPS/{row['href']}" in names
        assert present == (row["kind"] != "missing-from-zip"), row
        if row["linear"] and row["kind"] != "nav":
            linear += 1
    assert linear == truth["linear_chapters"] - sum(r["kind"] == "unresolved-idref" and r["linear"] for r in truth["spine"]), "linear count"
    for item in manifest.values():
        props = item.get("properties", "")
        if "nav" in props.split():
            found = []
            nav = ET.fromstring(archive.read(f"OEBPS/{item.get('href')}")).find(f"{XHTML_NS}body/{XHTML_NS}nav/{XHTML_NS}ol")
            nav_rows(nav, 0, found)
            assert found == truth["toc"], "nav toc"
        if item.get("media-type") == "application/x-dtbncx+xml":
            found = []
            ncx_rows(ET.fromstring(archive.read(f"OEBPS/{item.get('href')}")).find(f"{NCX_NS}navMap"), 0, found)
            assert found == truth["toc"], "ncx toc"


def check_text_book(truth, path, pool):
    book = truth["book"]
    with zipfile.ZipFile(path) as archive:
        assert archive.testzip() is None, f"{book}: CRC failure"
        order = archive.namelist()
        if book != "mimetype-not-first":
            assert order[0] == "mimetype" and archive.getinfo("mimetype").compress_type == zipfile.ZIP_STORED
        else:
            assert order[0] != "mimetype" and archive.getinfo("mimetype").compress_type == zipfile.ZIP_DEFLATED
        assert archive.read("mimetype") == b"application/epub+zip"
        check_package(truth, archive)
    terms = list(truth["terms"])
    jobs = []
    for row in truth["spine"]:
        if row["kind"] not in ("text", "image", "empty"):
            continue
        if row["href"] is None or row["href"].endswith(".png"):
            continue
        shape = None
        if truth["category"] == "large" and re.fullmatch(r"Text/c\d{5}\.xhtml", row["href"]):
            shape = "h1,p" if book == "large-h1-p" else "h1,div"
        jobs.append((str(path), f"OEBPS/{row['href']}", row["encoding"], terms, shape, row))
    results = pool.map(scan_chapter, [j[:5] for j in jobs], chunksize=64)
    totals = {term: 0 for term in terms}
    folded_totals = {term: 0 for term in terms}
    words = []
    for job, result in zip(jobs, results):
        row = job[5]
        assert result["well_formed"] == row["well_formed_after_decoding"], f"{book} {row['href']} well_formed {result['well_formed']}"
        assert result["shape_ok"], f"{book} {row['href']} shape"
        if row["kind"] in ("image", "empty"):
            assert result["words"] == 0, f"{book} {row['href']} words {result['words']}"
        ix = str(row["ix"])
        for term in terms:
            want = truth["terms"][term]["exact"].get(ix, 0)
            assert result["exact"][term] == want, f"{book} {row['href']} {term!r}: found {result['exact'][term]}, truth {want}"
            want = truth["terms"][term]["insensitive"].get(ix, 0)
            assert result["folded"][term] == want, f"{book} {row['href']} folded {term!r}: found {result['folded'][term]}, truth {want}"
            totals[term] += result["exact"][term]
            folded_totals[term] += result["folded"][term]
        if job[4]:
            words.append(result["words"])
    if truth["category"] == "large":
        assert len(words) == 6000, len(words)
        assert all(2700 <= w <= 3300 for w in words), (min(words), max(words))
        print(f"  {book}: {len(words)} chapters, words min {min(words)} max {max(words)} mean {sum(words) / len(words):.0f}")
        for term, facts in truth["terms"].items():
            if not facts["class"].startswith("zero"):
                assert facts["exact_total"] > 0, term
    for term, facts in truth["terms"].items():
        assert totals[term] == facts["exact_total"], term
        assert folded_totals[term] == facts["insensitive_total"], term
        if facts["class"].startswith("zero"):
            assert facts["exact_total"] == 0, term


def check_terms_are_real(truth, path):
    if truth["book"] != "gbk-xml-decl":
        return
    assert any(b < 0x80 for b in "丂亐乥".encode("gb18030")), "GBK term needs an ASCII trail byte"
    with zipfile.ZipFile(path) as archive:
        raw = archive.read("OEBPS/Text/c00.xhtml")
    assert raw.startswith(b'<?xml version="1.0" encoding="GBK"?>')
    assert any(b >= 0x80 for b in raw)


def check_meta_only(truth, path):
    if truth["book"] != "gbk-meta-charset":
        return
    with zipfile.ZipFile(path) as archive:
        for row in truth["spine"]:
            raw = archive.read(f"OEBPS/{row['href']}")
            assert b"<?xml" not in raw, "meta book must have no XML declaration"
            assert b'charset="gbk"' in raw or b"charset=gbk" in raw
            try:
                raw.decode("utf-8")
                raise AssertionError("meta chapter must not be valid UTF-8")
            except UnicodeDecodeError:
                pass


def check_attack(truth, path):
    port = truth["attack"]["probe_port"]
    rows = truth["attack"]["rows"]
    assert rows[0]["family"] == "control"
    ids = [r["id"] for r in rows]
    assert len(set(ids)) == len(ids)
    host = f"http://127.0.0.1:{port}"
    with zipfile.ZipFile(path) as archive:
        joined = b""
        for row in rows:
            chapter = truth["spine"][row["chapter"]]
            raw = archive.read(f"OEBPS/{chapter['href']}")
            joined += raw
            assert chapter["href"].endswith(f"/{row['id']}.xhtml"), row["id"]
            assert f"attack row {row['id']}".encode() in raw, row["id"]
            for probe in row["probes"]:
                text = raw + b"".join(archive.read(n) for n in archive.namelist() if n.startswith("OEBPS/Res/") and not n.endswith(".png"))
                assert (host + probe).encode() in text, (row["id"], probe)
            assert is_well_formed(raw, chapter["encoding"]) == row["well_formed"], row["id"]
        resources = b"".join(archive.read(n) for n in archive.namelist() if n.startswith("OEBPS/Res/") and not n.endswith(".png"))
    everything = (joined + resources).decode("utf-8", errors="replace")
    urls = set(re.findall(r"\bhttps?://[^\"'()\s,;<&?#]+", everything)) - {
        "http://www.w3.org/1999/xhtml", "http://www.w3.org/2000/svg", "http://www.w3.org/1999/xlink",
        "http://www.w3.org/1998/Math/MathML", "http://www.w3.org/2001/XInclude",
    }
    assert urls, "expected probe URLs"
    assert all(u.startswith(host + "/") for u in urls), [u for u in urls if not u.startswith(host + "/")][:5]
    families = {r["family"] for r in rows}
    payload_markers = ["<script", "onerror=", "srcdoc=", "<base ", "@import", "@font-face", "file:///", "javascript:", "java&#115;cript:", "__TAURI_INTERNALS__", "data-ss-orig-", "+ADw-script", "<!DOCTYPE html [", "<iframe", "<object", "<embed", "xml-stylesheet", "http-equiv=\"refresh\"", "<meta http-equiv=\"Content-Security-Policy\"", "formaction=", "ping="]
    missing = [m for m in payload_markers if m not in everything]
    assert not missing, missing
    print(f"  attack: {len(rows)} rows, families {len(families)}, {sum(len(r['probes']) for r in rows)} probe paths on port {port}")


def main():
    directory = Path(sys.argv[1])
    only = set(sys.argv[2:])
    truths = sorted(directory.glob("*.truth.json"))
    assert truths, f"no truth files in {directory}"
    failures = 0
    with multiprocessing.Pool() as pool:
        for truth_path in truths:
            truth = json.loads(truth_path.read_text(encoding="utf-8"))
            book = truth["book"]
            if only and book not in only:
                continue
            epub = directory / f"{book}.epub"
            try:
                if truth["category"] == "bomb":
                    check_bomb(truth, epub)
                else:
                    check_text_book(truth, epub, pool)
                    check_terms_are_real(truth, epub)
                    check_meta_only(truth, epub)
                if truth.get("attack"):
                    check_attack(truth, epub)
                print(f"ok {book}")
            except Exception as error:
                failures += 1
                print(f"FAILED {book}: {type(error).__name__}: {error}")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
