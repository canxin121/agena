#!/usr/bin/env python3
"""Compact inspectable outputs; avoid republishing complete public pages."""
import hashlib
import json
import sys
from pathlib import Path


def digest(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def main():
    fixtures_path, baselines_path, production_path, output_path = map(Path, sys.argv[1:])
    fixtures = json.loads(fixtures_path.read_text())
    public = {row["name"] for row in json.loads((Path(__file__).parent / "public-pages.json").read_text())}
    baselines = [json.loads(line) for line in baselines_path.read_text().splitlines()]
    for row in baselines:
        row["markdown_sha256"] = digest(row.get("markdown", ""))
        if row["fixture"] in public:
            row.pop("markdown", None)
            row.pop("comments", None)
    production = [json.loads(line) for line in production_path.read_text().splitlines()]
    for row in production:
        page = row.pop("page")
        row.update(title=page["title"], content_status=page["content_status"],
                   strategy=page["extraction_strategy"], warnings=page.get("warnings", []),
                   markdown_bytes=len(page["markdown"].encode()), markdown_sha256=digest(page["markdown"]))
        if row["fixture"] not in public:
            row["markdown"] = page["markdown"]
    evidence = {
        "as_of": "2026-10-05",
        "method": "Diagnostic fixtures, five standalone extraction pipelines and Agena production pipeline. Marker matching includes title, Markdown (only escaped underscores normalized), and separate comments. Not a representative quality/performance benchmark. Public fixtures reuse captured HTML; input hashes identify the actual observation.",
        "inputs": [{"name": row["name"], "url": row.get("url"), "html_sha256": digest(row["html"]), "facts": row["facts"], "noise": row["noise"]} for row in fixtures],
        "baselines": baselines,
        "production": production,
    }
    Path(output_path).write_text(json.dumps(evidence, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
