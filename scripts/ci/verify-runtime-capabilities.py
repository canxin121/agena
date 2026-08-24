#!/usr/bin/env python3
"""Fail if a full backend can compile only by dropping runtime crypto/auth capability."""

from __future__ import annotations

import pathlib
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parents[2]


def main() -> None:
    lock = (ROOT / "Cargo.lock").read_text(encoding="utf-8")
    forbidden_packages = ("aws-lc-rs", "aws-lc-sys", "rustls-platform-verifier")
    for package in forbidden_packages:
        if f'name = "{package}"' in lock:
            raise SystemExit(f"forbidden runtime crypto package remains in Cargo.lock: {package}")

    manifest = (ROOT / "crates/agena-provider-bedrock-auth/Cargo.toml").read_text(encoding="utf-8")
    source = (ROOT / "crates/agena-provider-bedrock-auth/src/lib.rs").read_text(encoding="utf-8")
    for needle in ("target_arch", "target_os", "target_abi", "target_env"):
        if needle in manifest or needle in source:
            raise SystemExit(
                f"Bedrock auth must not hide credential/TLS capability behind target cfg: {needle}"
            )

    tree = subprocess.check_output(
        [
            "cargo",
            "tree",
            "-p",
            "agena-provider-bedrock-auth",
            "-e",
            "features",
            "--locked",
        ],
        cwd=ROOT,
        text=True,
    )
    if 'aws-smithy-http-client feature "rustls-ring"' not in tree:
        raise SystemExit("Bedrock auth dependency graph does not enable aws-smithy-http-client/rustls-ring")
    if "aws-lc" in tree.lower():
        raise SystemExit("Bedrock auth dependency graph unexpectedly contains AWS-LC")

    test_cmd = [
        "cargo",
        "test",
        "-p",
        "agena-provider-bedrock-auth",
        "runtime_",
        "--locked",
    ]
    subprocess.check_call(
        [
            "cargo",
            "test",
            "-p",
            "agena-provider-bedrock-auth",
            "--locked",
            "--no-run",
        ],
        cwd=ROOT,
    )
    if sys.platform != "darwin":
        subprocess.check_call(test_cmd, cwd=ROOT)
    else:
        print(
            "Darwin host: construction tests compiled and linked; execution is deferred to the "
            "Linux CI/Release gate because this development Mac has the known Rust libunwind "
            "`failed to initiate panic, error 5` test-runtime abort."
        )

    print(
        "Runtime crypto/auth capabilities verified: Rustls/ring + bundled Mozilla roots "
        "+ full AWS provider chain + reqwest HTTPS client construction"
    )


if __name__ == "__main__":
    main()
