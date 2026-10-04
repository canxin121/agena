#!/usr/bin/env python3
"""Reproduce Git semantics behind Agena's prompt policy; no model evaluation.

All repositories, hooks and remotes live under a disposable temporary directory.
Git reads no personal/system configuration, and publication only targets local
bare fixture repositories. This is a research companion, not runtime machinery.
Run: python3 scripts/experiments/git_workflow.py
"""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class GitWorkflowProbes(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="agena-git-probes-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        self.env.update(
            GIT_CONFIG_NOSYSTEM="1",
            GIT_CONFIG_GLOBAL=os.devnull,
            GIT_TERMINAL_PROMPT="0",
            GIT_EDITOR="true",
        )
        self.template = self.root / "empty-template"
        self.template.mkdir()

    def command(self, repo, *args, ok=True):
        result = subprocess.run(
            [
                "git", "-c", "user.name=Agena Git Probe",
                "-c", "user.email=git-probe@example.invalid",
                "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false",
                "-c", "core.autocrlf=false", *args,
            ],
            cwd=repo, env=self.env, capture_output=True, text=True,
            timeout=20,
        )
        if ok:
            self.assertEqual(result.returncode, 0, f"git {args}: {result.stderr}")
        return result

    def git(self, repo, *args):
        return self.command(repo, *args).stdout.strip()

    def repository(self, name="repo", bare=False):
        repo = self.root / name
        repo.mkdir()
        options = ["--bare"] if bare else []
        self.git(repo, "init", "--quiet", "--initial-branch=main",
                 f"--template={self.template}", *options)
        return repo

    def commit(self, repo, name, content, message):
        (repo / name).write_text(content)
        self.git(repo, "add", "--", name)
        self.git(repo, "commit", "--quiet", "-m", message)
        return self.git(repo, "rev-parse", "HEAD")

    def test_path_commit_takes_unstaged_content_too(self):
        repo = self.repository()
        self.commit(repo, "shared.txt", "base\n", "base")
        (repo / "shared.txt").write_text("task\n")
        self.git(repo, "add", "--", "shared.txt")
        (repo / "shared.txt").write_text("task\nuser unstaged work\n")
        self.assertEqual(self.git(repo, "show", ":shared.txt"), "task")
        self.git(repo, "commit", "--quiet", "--only", "-m", "task", "--", "shared.txt")
        self.assertEqual(self.git(repo, "show", "HEAD:shared.txt"),
                         "task\nuser unstaged work")

    def test_plain_commit_consumes_the_whole_index(self):
        repo = self.repository()
        self.commit(repo, "user.txt", "base\n", "base")
        (repo / "user.txt").write_text("user staged work\n")
        self.git(repo, "add", "--", "user.txt")
        self.commit(repo, "task.txt", "task\n", "task")
        paths = self.git(repo, "diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD")
        self.assertEqual(set(paths.splitlines()), {"user.txt", "task.txt"})

    def test_working_tree_test_can_pass_while_staged_tree_fails(self):
        repo = self.repository()
        self.commit(repo, "answer.py", "answer = 0\n", "base")
        (repo / "answer.py").write_text("answer = 1\n")
        self.git(repo, "add", "--", "answer.py")
        (repo / "answer.py").write_text("answer = 2\n")
        for code, expected_success in [
            ((repo / "answer.py").read_text(), True),
            (self.git(repo, "show", ":answer.py"), False),
        ]:
            result = subprocess.run(
                [sys.executable, "-c", code + "\nassert answer == 2"],
                capture_output=True, timeout=20,
            )
            self.assertEqual(result.returncode == 0, expected_success)

    def test_worktree_separates_index_but_shares_config(self):
        repo = self.repository()
        base = self.commit(repo, "tracked.txt", "base\n", "base")
        (repo / "tracked.txt").write_text("staged user change\n")
        self.git(repo, "add", "--", "tracked.txt")
        linked = self.root / "linked"
        self.git(repo, "worktree", "add", "--quiet", "-b", "task", str(linked), base)
        self.assertTrue((linked / ".git").is_file())
        self.assertEqual((linked / "tracked.txt").read_text(), "base\n")
        self.assertEqual(self.git(linked, "diff", "--cached", "--name-only"), "")
        self.assertEqual(self.git(repo, "diff", "--cached", "--name-only"), "tracked.txt")
        self.git(repo, "config", "agena.probe", "shared")
        self.assertEqual(self.git(linked, "config", "--get", "agena.probe"), "shared")

    @unittest.skipIf(os.name == "nt", "fixture hooks use a POSIX shell")
    def test_hook_failure_does_not_always_mean_no_mutation(self):
        repo = self.repository()
        base = self.commit(repo, "file.txt", "base\n", "base")
        hooks = repo / ".git" / "hooks"
        hooks.mkdir()
        (repo / "file.txt").write_text("task\n")
        self.git(repo, "add", "--", "file.txt")
        pre = hooks / "pre-commit"
        pre.write_text("#!/bin/sh\nexit 7\n")
        pre.chmod(0o755)
        self.assertNotEqual(self.command(repo, "commit", "-m", "task", ok=False).returncode, 0)
        self.assertEqual(self.git(repo, "rev-parse", "HEAD"), base)
        pre.unlink()
        post = hooks / "post-commit"
        post.write_text("#!/bin/sh\necho post-commit-failed >&2\nexit 7\n")
        post.chmod(0o755)
        result = self.command(repo, "commit", "-m", "task")
        self.assertIn("post-commit-failed", result.stderr)
        self.assertNotEqual(self.git(repo, "rev-parse", "HEAD"), base)
        checkout = hooks / "post-checkout"
        checkout.write_text("#!/bin/sh\nexit 7\n")
        checkout.chmod(0o755)
        self.assertNotEqual(self.command(repo, "switch", "-c", "changed", ok=False).returncode, 0)
        self.assertEqual(self.git(repo, "branch", "--show-current"), "changed")

    def test_recovery_ref_does_not_save_dirty_files(self):
        repo = self.repository()
        self.commit(repo, "file.txt", "base\n", "base")
        (repo / "file.txt").write_text("uncommitted\n")
        self.git(repo, "branch", "recovery")
        self.assertEqual(self.git(repo, "show", "recovery:file.txt"), "base")
        self.assertEqual((repo / "file.txt").read_text(), "uncommitted\n")

    def test_clean_squash_preserves_tree_and_old_tip(self):
        repo = self.repository()
        base = self.commit(repo, "file.txt", "base\n", "base")
        self.commit(repo, "file.txt", "step one\n", "wip")
        old_tip = self.commit(repo, "file.txt", "finished\n", "fixup")
        old_tree = self.git(repo, "rev-parse", "HEAD^{tree}")
        self.git(repo, "branch", "recovery", old_tip)
        self.git(repo, "reset", "--soft", base)
        self.git(repo, "commit", "--quiet", "-m", "complete task")
        self.assertEqual(self.git(repo, "rev-parse", "HEAD^{tree}"), old_tree)
        self.assertEqual(self.git(repo, "rev-parse", "recovery"), old_tip)
        self.assertEqual(self.git(repo, "rev-list", "--count", f"{base}..HEAD"), "1")

    def test_explicit_lease_survives_background_fetch(self):
        remote = self.repository("remote.git", bare=True)
        local = self.repository("local")
        old_tip = self.commit(local, "base.txt", "base\n", "base")
        self.git(local, "remote", "add", "origin", str(remote))
        self.git(local, "push", "--quiet", "-u", "origin", "HEAD:refs/heads/main")
        other = self.root / "other"
        self.git(self.root, "clone", "--quiet", f"--template={self.template}", str(remote), str(other))
        other_tip = self.commit(other, "other.txt", "other person's work\n", "other")
        self.git(other, "push", "--quiet", "origin", "HEAD:refs/heads/main")
        local_tip = self.commit(local, "local.txt", "local work\n", "local")
        self.git(local, "fetch", "--quiet", "origin")
        self.assertEqual(self.git(local, "rev-parse", "origin/main"), other_tip)
        refused = self.command(local, "push", f"--force-with-lease=refs/heads/main:{old_tip}",
                               "origin", "HEAD:refs/heads/main", ok=False)
        self.assertNotEqual(refused.returncode, 0)
        self.assertEqual(self.git(remote, "rev-parse", "main"), other_tip)
        # In this disposable remote, prove that the implicit lease is now too
        # weak: a fetch advanced its expected value without integrating work.
        self.git(local, "push", "--quiet", "--force-with-lease", "origin", "HEAD:refs/heads/main")
        self.assertEqual(self.git(remote, "rev-parse", "main"), local_tip)

    def test_explicit_refspec_can_still_publish_to_multiple_urls_and_tags(self):
        repo = self.repository()
        self.commit(repo, "file.txt", "base\n", "base")
        remotes = [self.repository(name, bare=True) for name in ("one.git", "two.git")]
        self.git(repo, "remote", "add", "origin", str(remotes[0]))
        for remote in remotes:
            self.git(repo, "remote", "set-url", "--add", "--push", "origin", str(remote))
        self.git(repo, "config", "push.followTags", "true")
        self.git(repo, "tag", "-a", "checkpoint", "-m", "annotation")
        self.git(repo, "push", "--quiet", "origin", "HEAD:refs/heads/topic")
        self.assertEqual(len(self.git(repo, "remote", "get-url", "--push", "--all", "origin").splitlines()), 2)
        for remote in remotes:
            self.assertTrue(self.git(remote, "rev-parse", "refs/heads/topic"))
            self.assertTrue(self.git(remote, "rev-parse", "refs/tags/checkpoint"))

    def test_reusing_squash_merged_branch_repeats_old_commits(self):
        repo = self.repository()
        base = self.commit(repo, "base.txt", "base\n", "base")
        self.git(repo, "switch", "--quiet", "-c", "topic")
        first = self.commit(repo, "feature.txt", "first\n", "first feature")
        self.git(repo, "switch", "--quiet", "main")
        self.git(repo, "merge", "--squash", "topic")
        self.git(repo, "commit", "--quiet", "-m", "integrate feature")
        self.git(repo, "switch", "--quiet", "topic")
        second = self.commit(repo, "next.txt", "second\n", "next feature")
        self.assertEqual(self.git(repo, "merge-base", "main", "topic"), base)
        self.assertEqual(set(self.git(repo, "rev-list", "main..topic").splitlines()), {first, second})
        self.assertEqual(set(self.git(repo, "diff", "--name-only", "main...topic").splitlines()),
                         {"feature.txt", "next.txt"})


if __name__ == "__main__":
    unittest.main(verbosity=2)
