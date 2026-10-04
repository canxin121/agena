"""Extract already-fetched HTML locally; stdout is bounded JSON, never a traceback."""
import json
import sys

try:
    import trafilatura
except ImportError:
    print(json.dumps({"error": "missing_dependency"}))
    raise SystemExit(0)

try:
    html = sys.stdin.buffer.read(32 * 1024 * 1024 + 1)
    if len(html) > 32 * 1024 * 1024:
        result = {"error": "too_large"}
    else:
        text = trafilatura.extract(
            html.decode("utf-8"), output_format="markdown", include_comments=True,
            include_tables=True, include_links=True, include_images=False,
            with_metadata=False, favor_recall=False,
        )
        result = {"text": text} if text else {"error": "empty"}
        # Limit serialized bytes too (HTML can contain JSON escape characters).
        if len(json.dumps(result, ensure_ascii=False).encode("utf-8")) > 2 * 1024 * 1024:
            result = {"error": "too_large"}
except Exception:
    result = {"error": "conversion_failed"}
print(json.dumps(result, ensure_ascii=False))
