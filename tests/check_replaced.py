import sys
import xml.parsers.expat
import zipfile


def parses(data):
    parser = xml.parsers.expat.ParserCreate()
    parser.UseForeignDTD(True)
    try:
        parser.Parse(data, True)
    except xml.parsers.expat.ExpatError:
        return False
    return True


def main():
    old_path, new_path = sys.argv[1:3]
    changed = parsed_before = parsed_after = broke = 0
    with zipfile.ZipFile(old_path) as old, zipfile.ZipFile(new_path) as new:
        names = set(old.namelist())
        for name in new.namelist():
            if name not in names:
                continue
            before = old.read(name)
            after = new.read(name)
            if before == after:
                continue
            changed += 1
            was_good = parses(before)
            is_good = parses(after)
            parsed_before += was_good
            parsed_after += is_good
            broke += was_good and not is_good
    print(f"changed={changed} parsed_before={parsed_before} parsed_after={parsed_after} broke={broke}")


main()
