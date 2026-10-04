# Local document extraction and search

`fs.document` extracts text from one local PDF, DOCX, PPTX or XLSX file. Add a `pattern` to search the extracted lines. It uses optional mature converters and does not install dependencies when invoked.

```json
{"path":"reports/annual.pdf","pattern":"revenue","ignore_case":true,"max_lines":50}
```

For a text preview without filtering:

```json
{"path":"reports/annual.docx","start_line":1,"max_lines":100}
```

`fixed_strings` defaults to true. Set it to false for a Rust regex. Patterns preserve whitespace. `start_line` is a one-based line in the converted text; it is not a source page number. `max_lines` is 1–500. The payload includes the actual backend, source format and SHA-256, total extracted lines, matching lines in the requested range, line records, converter warnings and explicit truncation flags. `conversion_complete` means the converter finished within its transport limits; it does not certify that every chart, layout feature or scanned page was understood.

## Backends and setup

| Backend | Formats | Selection | Requirements |
| --- | --- | --- | --- |
| `auto` | PDF / Office | Uses `pdftotext` when available for PDF; otherwise selected MarkItDown converters | At least one applicable converter |
| `pdftotext` | PDF | Explicit Poppler text extraction with layout spacing and UTF-8 stdout | Poppler's `pdftotext` on the runtime PATH |
| `markitdown` | PDF, DOCX, PPTX, XLSX | Explicit converter class for the requested format | Python with MarkItDown's corresponding extras |

MarkItDown **0.1.8** was tested with Python 3.13. For a separate Python environment, for example:

```sh
uv venv --python 3.13 /path/to/document-env
uv pip install --python /path/to/document-env/bin/python 'markitdown[docx,pdf,pptx,xlsx]==0.1.8'
```

Start the Agena process with `AGENA_DOCUMENT_PYTHON=/path/to/document-env/bin/python` (on Windows, the interpreter is normally `Scripts/python.exe`). Without this setting, the adapter searches for host `python3`, or `python` on Windows. The environment must belong to the running Agena process; a separate interactive shell can have a different PATH. A `markitdown` CLI installed into its own isolated environment does not by itself make that package importable from host Python.

The adapter invokes only the selected PDF/Office converter, supplies a local file stream, and provides no LLM client or remote document-service configuration. It does not enable third-party MarkItDown plugins, URL conversion, audio transcription, image interpretation, or automatic format dispatch. Optional Python dependencies are separate from Agena's Rust installation.

## Bounds and interpretation

The source must be a regular file no larger than 32 MiB. A private temporary copy captures the exact bytes and SHA-256 sent to the converter; the source is not edited. Observed source changes while reading are rejected. At most two conversions run concurrently; managed process groups / Job Objects provide the existing timeout and cancellation cleanup behavior. Conversion has a 30-second process deadline and a 2 MiB cap on each output stream. Overflow fails explicitly rather than returning a silently incomplete conversion. Office archives are limited to 4,096 members and 64 MiB of declared expanded content.

Displayed line records have a combined 128 KiB bound. Individual lines can be shortened at 4 KiB and carry `text_truncated`. Counts cover the complete converted text even when fewer records are shown. Advance `start_line` or narrow the pattern for another range; use the selected converter directly when a complete very long line is needed. These are text-extraction bounds, not a general-purpose sandbox for arbitrary third-party Python packages.

An empty extraction can indicate a scanned PDF that needs OCR. Complex page order, tables, diagrams, scanned text and embedded media require inspection. Docling remains an optional layout/OCR workflow via the managed shell, and `rga` remains an optional multi-format directory search workflow; neither is silently invoked here. The adapter is intended for bounded text reading and search of a selected file.

## Reproduce validation

With the test environment installed, generate four small real files and run the existing local plugin-dispatch fixture:

```sh
/path/to/document-env/bin/python tools/document_adapter_fixtures.py /tmp/agena-document-fixtures
AGENA_DOCUMENT_PYTHON=/path/to/document-env/bin/python \
AGENA_DOCUMENT_FIXTURE_DIR=/tmp/agena-document-fixtures \
cargo test --locked -p agena-bundled-plugins --test tool_correctness \
  audit_local_documents_with_real_markitdown -- --ignored --nocapture
```

This verifies PDF text, Unicode Office text and matching through the actual plugin/executor route, without starting an Agena service or calling a connector. Ordinary unit tests cover literal/regex modes, case handling, line offsets, exact limits, Unicode clipping, output-byte limits and source-snapshot identity. Poppler was not installed on the development host, so its actual conversion quality/latency has not been measured in this work.
