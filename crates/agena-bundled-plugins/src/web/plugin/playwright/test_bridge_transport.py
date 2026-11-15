"""Protocol-boundary tests; no Playwright installation or browser is required."""
import ast
import os
from pathlib import Path
import unittest
from unittest.mock import patch


def load_transport():
    tree = ast.parse(Path(__file__).with_name("bridge.py").read_text())
    tree.body = [
        node for node in tree.body
        if isinstance(node, (ast.Import, ast.ImportFrom))
        or isinstance(node, ast.FunctionDef) and node.name == "sandbox_cdp_endpoint"
    ]
    namespace = {}
    exec(compile(tree, "bridge.py", "exec"), namespace)
    return namespace["sandbox_cdp_endpoint"]


transport = load_transport()


class SandboxTransportTests(unittest.TestCase):
    def test_namespace_mapping_preserves_owned_target_and_removes_proxy_environment(self):
        with patch.dict(os.environ, {
            "HTTP_PROXY": "http://10.0.2.2:9222/",
            "https_proxy": "http://old.invalid:9999/",
            "ALL_PROXY": "socks5://old.invalid:9999/",
            "NO_PROXY": "*",
        }, clear=True):
            endpoint = transport("ws://127.0.0.1:9222/devtools/browser/owned?token=private")
            self.assertEqual(
                endpoint,
                "ws://10.0.2.2:9222/devtools/browser/owned?token=private",
            )
            self.assertFalse(any(name.lower().endswith("_proxy") for name in os.environ))

    def test_native_loopback_mapping_keeps_the_exact_port(self):
        with patch.dict(os.environ, {"HTTP_PROXY": "http://127.0.0.1:9444/"}, clear=True):
            self.assertEqual(
                transport("ws://127.0.0.1:9444/devtools/browser/owned"),
                "ws://127.0.0.1:9444/devtools/browser/owned",
            )

    def test_default_http_port_survives_url_normalization(self):
        with patch.dict(os.environ, {"HTTP_PROXY": "http://10.0.2.2/"}, clear=True):
            self.assertEqual(
                transport("ws://127.0.0.1/devtools/browser/owned"),
                "ws://10.0.2.2/devtools/browser/owned",
            )

    def test_missing_or_changed_transport_is_rejected(self):
        for mapped in [
            None,
            "http://10.0.2.2:9333/",
            "http://192.0.2.1:9222/",
            "http://user:secret@10.0.2.2:9222/",
            "http://10.0.2.2:9222/another-service",
            "http://10.0.2.2:9222/?redirect=other",
            "socks5://10.0.2.2:9222/",
        ]:
            with self.subTest(mapped=mapped):
                environment = {} if mapped is None else {"HTTP_PROXY": mapped}
                with patch.dict(os.environ, environment, clear=True):
                    with self.assertRaises(ValueError):
                        transport("ws://127.0.0.1:9222/devtools/browser/owned")


if __name__ == "__main__":
    unittest.main()
