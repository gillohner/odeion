#!/usr/bin/env python3
"""Local regression checks for the emulator fixture's transport and acceptance gates.

No device, APK, node, credentials or Rust build needed. Subprocesses stand in for adb;
negative controls ensure a reachable but unauthenticated fixture cannot be accepted.
"""
import contextlib
import importlib.util
import io
import os
import subprocess
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("android_endpoint", Path(__file__).with_name("android-endpoint.py"))
ep = importlib.util.module_from_spec(spec)
with patch.object(sys, "argv", ["android-endpoint.py", "fixture.apk", "fixture-node", "/unused"]):
    spec.loader.exec_module(ep)


class TransportTests(unittest.TestCase):
    def test_guest_probe_uses_no_credentials_and_keeps_only_status(self):
        response = SimpleNamespace(returncode=0, stdout="HTTP/1.1 401 Unauthorized\r\n\r\nprivate response", stderr="")
        with patch.object(ep.subprocess, "run", return_value=response) as run:
            self.assertEqual(ep.guest_http_status("127.0.0.1", 17788), "401")
        request = run.call_args.kwargs["input"]
        self.assertIn("GET /v1/info HTTP/1.1\r\n", request)
        self.assertNotIn("authorization", request.lower())
        self.assertNotIn(ep.TOKEN, request)
        self.assertEqual(run.call_args.args[0][-2:], ["127.0.0.1", "17788"])

    def test_guest_probe_distinguishes_failures_without_response_disclosure(self):
        for response, expected in [
            (SimpleNamespace(returncode=0, stdout="not HTTP: private response", stderr=""), "no HTTP response"),
            (SimpleNamespace(returncode=1, stdout="", stderr="private error"), "exit 1"),
            (SimpleNamespace(returncode=0, stdout="HTTP/1.0 503 Unavailable\r\n\r\n", stderr=""), "503"),
        ]:
            with self.subTest(expected=expected), patch.object(ep.subprocess, "run", return_value=response):
                self.assertEqual(ep.guest_http_status("127.0.0.1", 17788), expected)
        with patch.object(ep.subprocess, "run", side_effect=subprocess.TimeoutExpired("adb", 10)):
            self.assertEqual(ep.guest_http_status("127.0.0.1", 17788), "timeout")

    def test_explicit_transport_requires_401_even_if_host_alias_fails(self):
        with patch.object(ep, "NODE", "http://127.0.0.1:7799"), \
                patch.object(ep, "adb") as adb, patch.object(ep, "log") as log, \
                patch.object(ep, "guest_http_status", side_effect=["timeout", "401"]) as probe:
            self.assertEqual(ep.prepare_node_transport(), "127.0.0.1:17788")
        adb.assert_called_once_with("reverse", "--no-rebind", "tcp:17788", "tcp:7799")
        self.assertEqual(probe.call_args_list[-1].args, ("127.0.0.1", 17788))
        self.assertIn("timeout", log.call_args_list[0].args[0])

    def test_reachable_open_or_broken_node_is_not_an_accepted_fixture(self):
        for status in ["200", "403", "404", "503", "timeout", "no HTTP response", "exit 1"]:
            with self.subTest(status=status), patch.object(ep, "NODE", "http://127.0.0.1:7788"), \
                    patch.object(ep, "adb"), patch.object(ep, "log"), \
                    patch.object(ep, "guest_http_status", side_effect=["401", status]):
                with self.assertRaisesRegex(RuntimeError, "must refuse an absent token with 401"):
                    ep.prepare_node_transport()

    def test_reverse_setup_failure_is_fatal(self):
        with patch.object(ep, "NODE", "http://localhost:7788"), patch.object(ep, "log"), \
                patch.object(ep, "guest_http_status", return_value="401"), \
                patch.object(ep, "adb", side_effect=RuntimeError("port already bound")):
            with self.assertRaisesRegex(RuntimeError, "port already bound"):
                ep.prepare_node_transport()

    def test_external_fixture_is_refused_before_adb(self):
        for node in ["https://localhost:7788", "http://example.com:7788"]:
            with self.subTest(node=node), patch.object(ep, "NODE", node), patch.object(ep, "adb") as adb:
                with self.assertRaisesRegex(RuntimeError, "runner's loopback"):
                    ep.prepare_node_transport()
                adb.assert_not_called()


class AcceptanceTests(unittest.TestCase):
    def test_enabled_tour_failure_propagates_for_both_enable_values(self):
        def failed(_):
            raise AssertionError("gesture did not open the sheet")

        tour = SimpleNamespace(run=failed)
        loader = SimpleNamespace(exec_module=lambda _: None)
        for mode in ["1", "strict"]:
            with self.subTest(mode=mode), patch.dict(os.environ, {"PM_UI_TOUR": mode}), \
                    patch("importlib.util.spec_from_file_location", return_value=SimpleNamespace(loader=loader)), \
                    patch("importlib.util.module_from_spec", return_value=tour):
                with self.assertRaisesRegex(AssertionError, "gesture did not open the sheet"):
                    ep.run_ui_tour()

    def test_disabled_tour_is_not_loaded(self):
        with patch.dict(os.environ, {"PM_UI_TOUR": ""}), patch("importlib.util.spec_from_file_location") as load:
            ep.run_ui_tour()
            load.assert_not_called()

    def test_adb_failure_and_timeout_never_expose_typed_token(self):
        token = "fixture-sensitive-value"
        with patch.object(ep, "TOKEN", token):
            for outcome in [SimpleNamespace(returncode=1, stdout=token, stderr=""),
                            subprocess.TimeoutExpired(["adb", "shell", "input", "text", token], 120)]:
                args = {"side_effect": outcome} if isinstance(outcome, Exception) else {"return_value": outcome}
                with patch.object(ep.subprocess, "run", **args):
                    with self.assertRaises(RuntimeError) as caught:
                        ep.adb("shell", "input", "text", token)
                    self.assertNotIn(token, str(caught.exception))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                ep.log(f"failed command with {token}")
            self.assertNotIn(token, output.getvalue())


if __name__ == "__main__":
    unittest.main(verbosity=2)
