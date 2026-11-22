import json
import sys
import xml.parsers.expat


def dump(document):
    events = []
    text = []

    def flush():
        if text:
            events.append("T" + "".join(text))
            text.clear()

    def start(name, attrs):
        flush()
        pairs = [f"{attrs[i]}={attrs[i + 1]}" for i in range(0, len(attrs), 2)]
        events.append("S" + name + "|" + "|".join(pairs))

    def end(name):
        flush()
        events.append("E" + name)

    parser = xml.parsers.expat.ParserCreate("utf-8")
    parser.ordered_attributes = True
    parser.buffer_text = False
    parser.StartElementHandler = start
    parser.EndElementHandler = end
    parser.CharacterDataHandler = text.append
    try:
        parser.Parse(document.encode("utf-8", "surrogatepass"), True)
    except (xml.parsers.expat.ExpatError, ValueError) as error:
        return {"well_formed": False, "events": [], "why": str(error)}
    flush()
    return {"well_formed": True, "events": events, "why": ""}


with open(sys.argv[1], encoding="utf-8") as source:
    documents = json.load(source)
with open(sys.argv[2], "w", encoding="utf-8") as out:
    json.dump([dump(document) for document in documents], out)
