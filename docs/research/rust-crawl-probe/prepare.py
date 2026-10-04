#!/usr/bin/env python3
"""Prepare offline fixtures, optionally adding bounded public-page snapshots."""

import argparse
import datetime
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parent
MAX_BYTES = 5 * 1024 * 1024


def read_bounded(path):
    with path.open("rb") as stream:
        data = stream.read(MAX_BYTES + 1)
    if len(data) > MAX_BYTES:
        raise ValueError("HTML exceeds the 5 MiB research fixture limit")
    return data


def download(url, path):
    result = subprocess.run(
        [
            "curl", "--fail", "--silent", "--show-error", "--location",
            "--proto", "=https", "--proto-redir", "=https",
            "--max-redirs", "5", "--connect-timeout", "10", "--max-time", "30",
            "--max-filesize", str(MAX_BYTES),
            "--user-agent", "agena-rust-web-research/1.0 (extraction comparison)",
            "--output", str(path), "--write-out", "%{http_code}\n%{url_effective}",
            url,
        ],
        check=True, capture_output=True, text=True, timeout=40,
    )
    status, final_url = result.stdout.splitlines()
    return read_bounded(path), int(status), final_url


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--live", action="store_true", help="Fetch the four recorded public URLs")
    source.add_argument("--html-dir", type=Path, help="Read NAME.html snapshots without network access")
    parser.add_argument("--allow-changed", action="store_true", help="Record changed snapshots instead of rejecting them")
    args = parser.parse_args()

    fixtures = json.loads((ROOT.parents[2] / "tools/fixtures/web-content.json").read_text(encoding="utf-8"))
    records = []
    if args.live or args.html_dir:
        pages = json.loads((ROOT / "public-pages.json").read_text(encoding="utf-8"))
        with tempfile.TemporaryDirectory(prefix="agena-web-probe-") as directory:
            for page in pages:
                if args.live:
                    data, status, final_url = download(
                        page["url"], Path(directory) / (page["name"] + ".html")
                    )
                else:
                    data = read_bounded(args.html_dir / (page["name"] + ".html"))
                    status, final_url = None, None
                digest = hashlib.sha256(data).hexdigest()
                changed = digest != page["expected_sha256"]
                if changed and not args.allow_changed:
                    raise ValueError(
                        page["name"] + ": snapshot differs from the report; "
                        "use --allow-changed to record a new observation"
                    )
                fixtures.append({
                    "name": page["name"], "url": page["url"],
                    "facts": page["facts"], "noise": page["noise"],
                    "html": data.decode("utf-8"),
                })
                records.append({
                    "name": page["name"], "url": page["url"],
                    "source": "live" if args.live else "local_snapshot",
                    "observed_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                    "http_status_this_run": status, "final_url_this_run": final_url,
                    "bytes": len(data), "sha256": digest,
                    "matches_report_snapshot": not changed,
                })

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(fixtures, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    provenance = args.output.with_suffix(".sources.json")
    provenance.write_text(json.dumps(records, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Prepared {len(fixtures)} fixtures at {args.output}", file=sys.stderr)
    print(f"Public snapshot provenance: {provenance}", file=sys.stderr)


if __name__ == "__main__":
    main()
