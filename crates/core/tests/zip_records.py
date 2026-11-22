import sys
import zipfile
from pathlib import Path


def records(path):
    data = Path(path).read_bytes()
    with zipfile.ZipFile(path) as archive:
        infos = archive.infolist()
        start_dir = archive.start_dir
    ordered = sorted(infos, key=lambda info: info.header_offset)
    extents = {}
    for ix, info in enumerate(ordered):
        end = ordered[ix + 1].header_offset if ix + 1 < len(ordered) else start_dir
        extents[info.filename] = data[info.header_offset : end]
    return extents, infos


def main():
    old_path, new_path, changed_path = sys.argv[1:4]
    changed = set(Path(changed_path).read_text(encoding="utf-8").split("\n")) - {""}
    old, _ = records(old_path)
    new, new_infos = records(new_path)
    same = differ_expected = differ_unexpected = removed = 0
    for name, record in old.items():
        if name not in new:
            removed += 1
        elif new[name] == record:
            same += 1
        elif name in changed:
            differ_expected += 1
        else:
            differ_unexpected += 1
    with zipfile.ZipFile(new_path) as archive:
        bad = archive.testzip()
    first = new_infos[0]
    print(
        f"same={same} differ_expected={differ_expected} differ_unexpected={differ_unexpected} "
        f"removed={removed} new_entries={len(new)} crc_failed={0 if bad is None else 1} "
        f"mimetype_first_stored={first.filename == 'mimetype' and first.compress_type == 0}"
    )


main()
