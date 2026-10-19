#!/usr/bin/env python3
"""Append a case to jq_1_8_1.jsonl, with what jq 1.8.1 itself answers.

    add_case.py '<document as JSON>' '<program>'

Needs `jq` 1.8.1 on the PATH. The case is added as it is: a document and a program for which jq
prints something (the lines of its output are kept, as text) or fails (then only that is kept).
Run the tests of the query crate afterwards: a case that this app does not answer as jq does is
a known issue, or a bug, and a known issue is no case for this file (it goes in
crates/query/src/known_issues/list.rs).
"""
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
FILE = os.path.join(HERE, "jq_1_8_1.jsonl")


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    document, program = sys.argv[1], sys.argv[2]
    json.loads(document)  # it must be JSON
    version = subprocess.run(["jq", "--version"], capture_output=True, text=True).stdout.strip()
    if version != "jq-1.8.1":
        sys.exit(f"these cases are those of jq-1.8.1, and this is {version}")
    run = subprocess.run(["jq", "-c", "-M", "--", program], input=document + "\n", capture_output=True, text=True)
    if run.returncode != 0:
        line = '{"in":%s,"q":%s,"error":true}' % (document, json.dumps(program, ensure_ascii=False))
    else:
        outputs = [l for l in run.stdout.split("\n") if l]
        line = '{"in":%s,"q":%s,"out":%s}' % (document, json.dumps(program, ensure_ascii=False), json.dumps(outputs, ensure_ascii=False))
    with open(FILE, "a", encoding="utf-8") as file:
        file.write(line + "\n")
    print(line)


main()
