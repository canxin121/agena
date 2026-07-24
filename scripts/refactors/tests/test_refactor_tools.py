from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import tempfile
import textwrap
import unittest


REPOSITORY_ROOT = pathlib.Path(__file__).resolve().parents[3]
TOOLS = REPOSITORY_ROOT / "scripts" / "refactors"


class RefactorToolTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temporary.name)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_tool(self, tool: str, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(TOOLS / tool), "--root", str(self.root), *arguments],
            cwd=REPOSITORY_ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def write(self, relative: str, content: str) -> pathlib.Path:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        return path

    def write_json(self, relative: str, value: object) -> pathlib.Path:
        return self.write(relative, json.dumps(value, indent=2) + "\n")

    def test_split_rust_file_dry_run_then_apply(self) -> None:
        source_text = textwrap.dedent(
            """\
            #![allow(dead_code)]
            //! File-level documentation stays in the module root.

            use std::fmt;

            /// Documentation moves with Widget.
            #[derive(Debug)]
            pub struct Widget {
                value: usize,
            }

            impl Widget {
                pub fn new(value: usize) -> Self {
                    Self { value }
                }
            }

            pub fn keep_in_source() -> impl fmt::Debug {
                7usize
            }

            #[cfg(test)]
            mod tests {
                #[test]
                fn smoke() {
                    assert_eq!(super::Widget::new(1).value, 1);
                }
            }
            """
        )
        source = self.write("src/widget.rs", source_text)
        manifest = self.write_json(
            "split.json",
            {
                "version": 1,
                "source": "src/widget.rs",
                "destinations": [
                    {
                        "path": "src/widget/types.rs",
                        "header": "use super::*;",
                        "items": ["struct:Widget", "impl@1"],
                    }
                ],
            },
        )

        inventory = self.run_tool(
            "split-rust-file.py", "list", "--source", "src/widget.rs"
        )
        self.assertEqual(inventory.returncode, 0, inventory.stderr)
        self.assertIn("struct:Widget", inventory.stdout)
        self.assertIn("impl@1", inventory.stdout)
        self.assertIn("mod:tests", inventory.stdout)

        dry_run = self.run_tool(
            "split-rust-file.py", "split", "--manifest", str(manifest)
        )
        self.assertEqual(dry_run.returncode, 0, dry_run.stderr)
        self.assertIn("dry-run only", dry_run.stdout)
        self.assertEqual(source.read_text(encoding="utf-8"), source_text)
        self.assertFalse((self.root / "src/widget/types.rs").exists())

        applied = self.run_tool(
            "split-rust-file.py", "split", "--manifest", str(manifest), "--apply"
        )
        self.assertEqual(applied.returncode, 0, applied.stderr)
        destination_text = (self.root / "src/widget/types.rs").read_text(encoding="utf-8")
        remaining = source.read_text(encoding="utf-8")
        self.assertIn("use super::*;", destination_text)
        self.assertIn("/// Documentation moves with Widget.", destination_text)
        self.assertIn("#[derive(Debug)]", destination_text)
        self.assertIn("pub struct Widget", destination_text)
        self.assertIn("impl Widget", destination_text)
        self.assertNotIn("pub struct Widget", remaining)
        self.assertNotIn("impl Widget", remaining)
        self.assertIn("#![allow(dead_code)]", remaining)
        self.assertIn("//! File-level documentation", remaining)
        self.assertIn("pub fn keep_in_source", remaining)
        self.assertIn("mod tests", remaining)

    def test_split_unknown_selector_writes_nothing(self) -> None:
        source = self.write("src/lib.rs", "pub struct Present;\n")
        before = source.read_text(encoding="utf-8")
        manifest = self.write_json(
            "invalid-split.json",
            {
                "version": 1,
                "source": "src/lib.rs",
                "destinations": [
                    {"path": "src/moved.rs", "items": ["struct:Missing"]}
                ],
            },
        )
        result = self.run_tool(
            "split-rust-file.py", "split", "--manifest", str(manifest), "--apply"
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("unknown selector", result.stderr)
        self.assertEqual(source.read_text(encoding="utf-8"), before)
        self.assertFalse((self.root / "src/moved.rs").exists())

    def test_split_existing_destination_is_never_overwritten(self) -> None:
        source = self.write("src/lib.rs", "pub struct Present;\n")
        destination = self.write("src/moved.rs", "// user-owned destination\n")
        manifest = self.write_json(
            "existing-destination.json",
            {
                "version": 1,
                "source": "src/lib.rs",
                "destinations": [
                    {"path": "src/moved.rs", "items": ["struct:Present"]}
                ],
            },
        )

        result = self.run_tool(
            "split-rust-file.py", "split", "--manifest", str(manifest), "--apply"
        )

        self.assertEqual(result.returncode, 2)
        self.assertIn("destination already exists", result.stderr)
        self.assertEqual(source.read_text(encoding="utf-8"), "pub struct Present;\n")
        self.assertEqual(
            destination.read_text(encoding="utf-8"), "// user-owned destination\n"
        )

    def test_assert_replace_is_dry_run_and_count_asserted(self) -> None:
        first = self.write("a.txt", "old old\n")
        second = self.write("b.txt", "old\n")
        manifest = self.write_json(
            "replace.json",
            {
                "version": 1,
                "replacements": [
                    {
                        "files": ["a.txt", "b.txt"],
                        "old": "old",
                        "new": "new",
                        "expected": 3,
                        "expected_new_before": 0,
                    }
                ],
            },
        )
        dry_run = self.run_tool(
            "assert-replace.py", "--manifest", str(manifest)
        )
        self.assertEqual(dry_run.returncode, 0, dry_run.stderr)
        self.assertEqual(first.read_text(encoding="utf-8"), "old old\n")
        self.assertEqual(second.read_text(encoding="utf-8"), "old\n")

        applied = self.run_tool(
            "assert-replace.py", "--manifest", str(manifest), "--apply"
        )
        self.assertEqual(applied.returncode, 0, applied.stderr)
        self.assertEqual(first.read_text(encoding="utf-8"), "new new\n")
        self.assertEqual(second.read_text(encoding="utf-8"), "new\n")

        mismatch = self.write_json(
            "mismatch.json",
            {
                "version": 1,
                "replacements": [
                    {
                        "files": ["a.txt", "b.txt"],
                        "old": "new",
                        "new": "later",
                        "expected": 2,
                    }
                ],
            },
        )
        before_first = first.read_text(encoding="utf-8")
        before_second = second.read_text(encoding="utf-8")
        failed = self.run_tool(
            "assert-replace.py", "--manifest", str(mismatch), "--apply"
        )
        self.assertEqual(failed.returncode, 2)
        self.assertIn("expected 2", failed.stderr)
        self.assertEqual(first.read_text(encoding="utf-8"), before_first)
        self.assertEqual(second.read_text(encoding="utf-8"), before_second)

        duplicate = self.write_json(
            "duplicate-file.json",
            {
                "version": 1,
                "replacements": [
                    {
                        "files": ["a.txt", "a.txt"],
                        "old": "new",
                        "new": "later",
                        "expected": 4,
                    }
                ],
            },
        )
        failed = self.run_tool(
            "assert-replace.py", "--manifest", str(duplicate), "--apply"
        )
        self.assertEqual(failed.returncode, 2)
        self.assertIn("duplicate path", failed.stderr)
        self.assertEqual(first.read_text(encoding="utf-8"), before_first)

    def test_invariant_manifest_passes_and_reports_violations(self) -> None:
        source = self.write("src/lib.rs", "pub fn ready() {}\n")
        manifest = self.write_json(
            "gates.json",
            {
                "version": 1,
                "must_exist": ["src/lib.rs"],
                "must_not_exist": ["old-owner"],
                "text_rules": [
                    {
                        "name": "old identity",
                        "roots": ["src"],
                        "include": ["**/*.rs"],
                        "pattern": "OLD_IDENTITY",
                        "expected": 0,
                    }
                ],
                "line_rules": [
                    {
                        "name": "small Rust sources",
                        "roots": ["src"],
                        "include": ["**/*.rs"],
                        "max_lines": 3,
                    }
                ],
            },
        )
        passed = self.run_tool(
            "check-refactor-invariants.py", "--manifest", str(manifest)
        )
        self.assertEqual(passed.returncode, 0, passed.stderr)
        self.assertIn("all 4 refactor invariant check(s) passed", passed.stdout)

        source.write_text("OLD_IDENTITY\nline two\nline three\nline four\n", encoding="utf-8")
        failed = self.run_tool(
            "check-refactor-invariants.py", "--manifest", str(manifest)
        )
        self.assertEqual(failed.returncode, 1)
        self.assertIn("text rule 'old identity'", failed.stderr)
        self.assertIn("line rule 'small Rust sources'", failed.stderr)


if __name__ == "__main__":
    unittest.main()
