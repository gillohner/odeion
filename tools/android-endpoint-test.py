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
    def setUp(self):
        self.route = "10.0.2.2 dev wlan0 src 10.0.2.16 uid 2000\n"
        self.reverse_state = patch.object(ep, "reverse_installed", False)
        self.reverse_state.start()
        self.addCleanup(self.reverse_state.stop)

    def guest_process(self, script):
        """An actual pipe-backed child, standing in for adb/nc; no sockets or device."""
        popen = subprocess.Popen
        return patch.object(ep.subprocess, "Popen", side_effect=lambda *_, **kw:
                            popen([sys.executable, "-c", script], **kw))

    def test_guest_probe_uses_no_credentials_and_keeps_only_status(self):
        script = '''import sys
request = b"".join(sys.stdin.buffer.readline() for _ in range(4))
assert request.startswith(b"GET /v1/info HTTP/1.1\\r\\n")
assert b"authorization" not in request.lower()
assert b"Host: 127.0.0.1:17788\\r\\n" in request
print("HTTP/1.1 401 Unauthorized\\r\\n\\r\\nprivate response", flush=True)
'''
        with self.guest_process(script) as run:
            self.assertEqual(ep.guest_http_status("127.0.0.1", 17788), "401")
        self.assertEqual(run.call_args.args[0][-2:], ["127.0.0.1", "17788"])

    def test_guest_stdin_stays_open_until_the_reply(self):
        # Model adb reverse's teardown on EOF. subprocess.run(input=...) closes stdin,
        # so the old preflight deterministically loses the reply from this same child.
        script = '''import os, select, sys
request = b""
while not request.endswith(b"\\r\\n\\r\\n"):
    request += os.read(0, 1)
if not select.select([0], [], [], .1)[0]:
    print("HTTP/1.1 401 Unauthorized\\r\\n\\r\\n", flush=True)
'''
        old = subprocess.run([sys.executable, "-c", script], input="GET /v1/info HTTP/1.1\r\n\r\n",
                             capture_output=True, text=True, timeout=3)
        self.assertEqual(old.stdout, "", "negative control: EOF destroys the reply")
        with self.guest_process(script):
            self.assertEqual(ep.guest_http_status("127.0.0.1", 17788), "401")

    def test_guest_probe_distinguishes_failures_without_response_disclosure(self):
        for script, expected in [
            ('print("not HTTP: private response")', "no HTTP response"),
            ('import sys; print("private error", file=sys.stderr); sys.exit(1)', "exit 1"),
            ('print("HTTP/1.0 503 Unavailable\\r\\n\\r\\n")', "503"),
        ]:
            with self.subTest(expected=expected), self.guest_process(script):
                self.assertEqual(ep.guest_http_status("127.0.0.1", 17788), expected)
        with self.guest_process('import time; time.sleep(30)'), \
                patch.object(ep.time, "monotonic", side_effect=[0, 11]):
            self.assertEqual(ep.guest_http_status("127.0.0.1", 17788), "timeout")

    def test_explicit_transport_requires_401_even_if_host_alias_fails(self):
        with patch.object(ep, "NODE", "http://127.0.0.1:7799"), \
                patch.object(ep, "adb", return_value=self.route) as adb, patch.object(ep, "log") as log, \
                patch.object(ep, "guest_http_status", side_effect=["timeout", "401"]) as probe:
            self.assertEqual(ep.prepare_node_transport(), "10.0.2.16:17788")
        adb.assert_any_call("reverse", "--no-rebind", "tcp:17788", "tcp:7799")
        self.assertEqual(probe.call_args_list[-1].args, ("10.0.2.16", 17788))
        self.assertIn("timeout", log.call_args_list[0].args[0])

    def test_reachable_open_or_broken_node_is_not_an_accepted_fixture(self):
        for status in ["200", "403", "404", "503", "timeout", "no HTTP response", "exit 1"]:
            with self.subTest(status=status), patch.object(ep, "NODE", "http://127.0.0.1:7788"), \
                    patch.object(ep, "adb", return_value=self.route), patch.object(ep, "log"), \
                    patch.object(ep, "guest_http_status", side_effect=["401", status]):
                with self.assertRaisesRegex(RuntimeError, "must refuse an absent token with 401"):
                    ep.prepare_node_transport()

    def test_reverse_setup_failure_is_fatal(self):
        with patch.object(ep, "NODE", "http://localhost:7788"), patch.object(ep, "log"), \
                patch.object(ep, "guest_http_status", return_value="401"), \
                patch.object(ep, "adb", side_effect=[self.route, RuntimeError("port already bound")]):
            with self.assertRaisesRegex(RuntimeError, "port already bound"):
                ep.prepare_node_transport()
            self.assertFalse(ep.reverse_installed)

    def test_external_fixture_is_refused_before_adb(self):
        for node in ["https://localhost:7788", "http://example.com:7788"]:
            with self.subTest(node=node), patch.object(ep, "NODE", node), patch.object(ep, "adb") as adb:
                with self.assertRaisesRegex(RuntimeError, "runner's loopback"):
                    ep.prepare_node_transport()
                adb.assert_not_called()

    def test_loopback_or_missing_guest_route_is_not_accepted(self):
        for route in ["", "local 10.0.2.2 dev lo src 127.0.0.1", "src 0.0.0.0", "src 8.8.8.8"]:
            with self.subTest(route=route), patch.object(ep, "adb", return_value=route) as adb, \
                    patch.object(ep, "NODE", "http://localhost:7788"):
                with self.assertRaises(RuntimeError):
                    ep.prepare_node_transport()
                self.assertFalse(ep.reverse_installed)
                self.assertEqual(adb.call_count, 1, "no reverse mapping on an invalid route")

    def test_cleanup_removes_only_this_runs_mapping(self):
        with patch.object(ep, "adb") as adb:
            ep.cleanup_node_transport()
            adb.assert_not_called()
            ep.reverse_installed = True
            ep.cleanup_node_transport()
            adb.assert_called_once_with("reverse", "--remove", "tcp:17788", check=False)
            ep.cleanup_node_transport()
            self.assertEqual(adb.call_count, 1)


class AcceptanceTests(unittest.TestCase):
    def test_first_registration_failure_is_not_hidden_by_later_success(self):
        failed = "WARN player not registered (the node must reach this machine): refused address=10.0.2.16:7790"
        passed = "INFO player registered with the node address=10.0.2.16:7790 outputs=1"
        with patch.object(ep, "logcat", return_value=failed + "\n" + passed):
            self.assertEqual(ep.registered(), (False, "10.0.2.16:7790"))
        with patch.object(ep, "logcat", return_value=passed):
            self.assertEqual(ep.registered(), (True, "10.0.2.16:7790"))
        with patch.object(ep, "logcat", return_value="WARN no player: cannot listen\n" + passed):
            with self.assertRaisesRegex(RuntimeError, "cannot listen"):
                ep.registered()

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
