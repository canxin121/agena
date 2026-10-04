#!/usr/bin/env python3
"""Opt-in Cargo-log experiment; never installed as a shell/CLI proxy.

Retains exact stdout/stderr and process return code, plus an inspectable preview.
Only successful libtest rows and Cargo compilation progress are folded. All
other bytes, including warnings/failures/unknown lines, remain in the preview.
Byte reduction is not a tokenizer, billing or model-success measurement.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

FOLD = re.compile(rb"(?:test [A-Za-z0-9_:]+ \.\.\. ok|[ \t]*(?:Compiling|Checking) [^\r\n]+)\r?\n\Z")


def summarize(raw):
    kept = []
    folded = 0
    for line in raw.splitlines(keepends=True):
        if FOLD.fullmatch(line):
            folded += 1
        else:
            kept.append(line)
    if folded:
        kept.append(f"\n[{folded} successful test/progress rows folded; exact bytes retained in raw log.]\n".encode())
    return b"".join(kept), folded


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("supply a command after --")
    args.output.mkdir(parents=True, exist_ok=False)
    with (args.output / "stdout.raw").open("wb") as out, (args.output / "stderr.raw").open("wb") as err:
        process = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=out, stderr=err, check=False)
    streams = {}
    for stream in ["stdout", "stderr"]:
        raw = (args.output / f"{stream}.raw").read_bytes()
        preview, folded = summarize(raw)
        (args.output / f"{stream}.preview").write_bytes(preview)
        streams[stream] = {"raw_bytes":len(raw), "preview_bytes":len(preview), "folded_lines":folded,
                           "raw_sha256":hashlib.sha256(raw).hexdigest(), "preview_sha256":hashlib.sha256(preview).hexdigest()}
    metadata = {"argv":command, "returncode":process.returncode, "streams":streams,
                "scope":"Cargo/libtest presentation experiment only; raw streams retained. No RTK binary, tokenizer or model task-success measurement."}
    (args.output / "result.json").write_text(json.dumps(metadata, indent=2)+"\n")
    print(json.dumps(metadata, ensure_ascii=False))
    # Preserve ordinary exit codes; report signal termination conventionally
    # while retaining the exact negative subprocess returncode in result.json.
    return process.returncode if process.returncode >= 0 else 128-process.returncode


if __name__ == "__main__":
    sys.exit(main())
