#!/usr/bin/env python3
"""Split Tree-sitter's generated lexer for the legacy C-SKY assembler.

The generated ``ts_lex`` function is a state machine.  Older C-SKY GNU
assemblers cannot encode branches between labels more than 64 KiB apart and
do not provide a usable linker relaxation for those branches.  This helper
keeps the generated state machine and lexer macros intact, but partitions the
state switch into small functions.  A small outer loop carries the lexer
state across partitions, so this is a code-layout change rather than a
grammar or parser substitute.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path


MAX_CHUNK_BYTES = 48_000
MARKER = "/* AGENA_CSKY_SPLIT_LEXER */"
LEXER_SIGNATURE = "static bool ts_lex(TSLexer *lexer, TSStateId state) {"


def _step(done: str, result: str, state: str, skip: str, resume: str) -> str:
    return f"(AgenaCskyLexStep){{{done}, {result}, {state}, {skip}, {resume}}}"


def transform(text: str) -> str | None:
    if MARKER in text or LEXER_SIGNATURE not in text:
        return None

    start = text.index(LEXER_SIGNATURE)
    next_declaration = re.search(
        r"\n}\n\n(?=static (?:bool ts_lex_keywords|const TS(?:Lexer|Lex)Mode ts_lex_modes))",
        text[start:],
    )
    if next_declaration is None:
        raise ValueError("Tree-sitter lexer has no following declaration")
    end = start + next_declaration.start() + 2
    function = text[start:end]

    cases = list(re.finditer(r"^    case (\d+):\n", function, re.MULTILINE))
    if not cases:
        raise ValueError("Tree-sitter lexer has no state cases")
    default = function.rfind("    default:")
    if default < cases[-1].end():
        raise ValueError("Tree-sitter lexer has no default state")

    case_ranges: list[tuple[int, int, int]] = []
    for index, match in enumerate(cases):
        case_end = cases[index + 1].start() if index + 1 < len(cases) else default
        case_ranges.append((int(match.group(1)), match.start(), case_end))

    chunks: list[tuple[list[int], str]] = []
    states: list[int] = []
    chunk_start: int | None = None
    chunk_end = 0
    for state, case_start, case_end in case_ranges:
        if chunk_start is None:
            chunk_start = case_start
        proposed_size = len(function[chunk_start:case_end].encode("utf-8"))
        if states and proposed_size > MAX_CHUNK_BYTES:
            chunks.append((states, function[chunk_start:chunk_end]))
            states = []
            chunk_start = case_start
        states.append(state)
        chunk_end = case_end
    if not states or chunk_start is None:
        raise ValueError("Tree-sitter lexer state partitioning produced no chunks")
    chunks.append((states, function[chunk_start:chunk_end]))

    generated: list[str] = [
        MARKER,
        "/* The partition boundary is only an assembler branch-range workaround. */",
        "typedef struct {",
        "  bool done;",
        "  bool result;",
        "  TSStateId state;",
        "  bool skip;",
        "  bool resume;",
        "} AgenaCskyLexStep;",
        "",
    ]

    for index, (states_in_chunk, body) in enumerate(chunks):
        first = states_in_chunk[0]
        last = states_in_chunk[-1]
        body = body.replace(
            "END_STATE();",
            f"return {_step('true', 'result', 'state', 'skip', 'false')};",
        )
        generated.extend(
            [
                f"static AgenaCskyLexStep ts_lex_csky_chunk_{index}(",
                "    TSLexer *lexer, TSStateId state, bool result, bool skip,",
                "    bool eof, bool resume) {",
                "  int32_t lookahead;",
                "  if (resume) goto next_state;",
                "  goto start;",
                "",
                "next_state:",
                f"  if (state < {first} || state > {last}) {{",
                f"    return {_step('false', 'result', 'state', 'skip', 'true')};",
                "  }",
                "  lexer->advance(lexer, skip);",
                "",
                "start:",
                "  skip = false;",
                "  lookahead = lexer->lookahead;",
                "  switch (state) {",
                body.rstrip("\n"),
                "    default:",
                f"      return {_step('true', 'false', 'state', 'skip', 'false')};",
                "  }",
                "}",
                "",
            ]
        )

    generated.extend(
        [
            LEXER_SIGNATURE,
            "  bool result = false;",
            "  bool skip = false;",
            "  bool eof = lexer->eof(lexer);",
            "  bool resume = false;",
            "",
            "  for (;;) {",
            "    AgenaCskyLexStep step;",
            "    switch (state) {",
        ]
    )
    for index, (states_in_chunk, _body) in enumerate(chunks):
        for state in states_in_chunk:
            generated.append(f"      case {state}:")
        generated.extend(
            [
                f"        step = ts_lex_csky_chunk_{index}(lexer, state, result, skip, eof, resume);",
                "        break;",
            ]
        )
    generated.extend(
        [
            "      default:",
            "        return false;",
            "    }",
            "    if (step.done) return step.result;",
            "    state = step.state;",
            "    result = step.result;",
            "    skip = step.skip;",
            "    resume = step.resume;",
            "  }",
            "}",
            "",
        ]
    )

    return text[:start] + "\n".join(generated) + text[end:]


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        raise SystemExit(f"usage: {argv[0]} INPUT OUTPUT")
    source = Path(argv[1])
    output = Path(argv[2])
    original = source.read_text(encoding="utf-8")
    transformed = transform(original)
    output.write_text(transformed if transformed is not None else original, encoding="utf-8")
    if transformed is not None:
        states = len(re.findall(r"^    case (\d+):$", transformed, re.MULTILINE))
        chunks = len(re.findall(r"^static AgenaCskyLexStep ts_lex_csky_chunk_", transformed, re.MULTILINE))
        print(f"Split Tree-sitter ts_lex: {states} states into {chunks} C-SKY-safe chunks", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
