"""Regenerate crates/debut-media/src/aaf/baseline.json.

The AAF writer (debut_media::aaf) stores the baseline MetaDictionary (every
class and type definition of the AAF object model) and the Dictionary (data,
operation and other definitions) exactly as pyaaf2 writes them, so files
carry the full model like the AAF SDK's do. This script asks pyaaf2
(https://github.com/markreidvfx/pyaaf2, MIT) to build an empty file, adds
the operation definitions the writer refers to, and dumps both object trees
generically: class ids, raw property bytes and references.

    python3 -m venv venv && venv/bin/pip install pyaaf2==1.7.1
    venv/bin/python tools/aaf_baseline.py crates/debut-media/src/aaf/baseline.json
"""
import io
import json
import sys

import aaf2
from aaf2 import properties as P

# Operation definitions for transitions (AAF edit protocol ids).
OPERATIONS = [
    ("0c3bea40-fc05-11d2-8a29-0050040ef7d2", "VideoDissolve_2", "picture"),
    ("0c3bea44-fc05-11d2-8a29-0050040ef7d2", "MonoAudioDissolve", "sound"),
]


def dump(o):
    props = []
    for pid, p in sorted(o.property_entries.items()):
        f = p.format
        if f == P.SF_DATA:
            props.append([pid, "d", p.data.hex()])
        elif f == P.SF_STRONG_OBJECT_REFERENCE:
            props.append([pid, "s", p.ref, dump(p.value)])
        elif f == P.SF_STRONG_OBJECT_REFERENCE_VECTOR:
            props.append([pid, "v", p.index_name, [dump(v) for v in p.value]])
        elif f == P.SF_STRONG_OBJECT_REFERENCE_SET:
            items = [[k.bytes_le.hex(), dump(v)] for k, v in p.items()]
            props.append([pid, "S", p.index_name, p.key_pid, p.key_size, items])
        elif f == P.SF_WEAK_OBJECT_REFERENCE:
            path = o.root.weakref_table[p.weakref_index]
            props.append([pid, "w", path, p.key_pid, p.ref.bytes_le.hex()])
        elif f in (P.SF_WEAK_OBJECT_REFERENCE_VECTOR, P.SF_WEAK_OBJECT_REFERENCE_SET):
            kind = "W" if f == P.SF_WEAK_OBJECT_REFERENCE_VECTOR else "WS"
            path = o.root.weakref_table[p.weakref_index]
            refs = [r.bytes_le.hex() for r in p.references]
            props.append([pid, kind, p.index_name, path, p.key_pid, p.key_size, refs])
        else:
            raise ValueError("unsupported stored form %x" % f)
    return {"c": o.dir.class_id.bytes_le.hex(), "p": props}


def main(out):
    buf = io.BytesIO()
    with aaf2.open(None, "w") as f:
        for auid, name, kind in OPERATIONS:
            op = f.create.OperationDef(auid, name, "")
            op.media_kind = kind
            op["NumberInputs"].value = 2
            f.dictionary.register_def(op)
        f.save()
        data = {
            "pyaaf2": aaf2.__version__ if hasattr(aaf2, "__version__") else "",
            "metadict": dump(f.metadict),
            "dictionary": dump(f.dictionary),
        }
    with open(out, "w") as fh:
        json.dump(data, fh, separators=(",", ":"))
        fh.write("\n")


if __name__ == "__main__":
    main(sys.argv[1])
