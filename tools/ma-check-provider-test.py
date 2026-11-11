#!/usr/bin/env python3
"""Offline checks of tools/ma-check.py's provider handling, against a stubbed MA API (no MA, no
node): a provider MA has already, one set up, the single-instance race MA's own start-up can
cause, and errors that must stay errors.

    python3 -I tools/ma-check-provider-test.py
"""
import importlib.util
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # nothing written next to the checked script

spec = importlib.util.spec_from_file_location("ma_check", Path(__file__).with_name("ma-check.py"))
ma_check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ma_check)


class Stub(ma_check.MusicAssistant):
    """The API answers from `configs` (what `config/providers` lists, in turn) and `setup`
    (what `config/providers/setup` does: a step, or an error to raise)."""

    def __init__(self, configs, setup):
        self.configs, self.setup, self.calls = list(configs), setup, []

    def api(self, command, **args):
        self.calls.append(command)
        if command == "config/providers":
            assert args == {"provider_domain": "dlna"}, args
            return self.configs.pop(0) if len(self.configs) > 1 else self.configs[0]
        if command == "config/providers/setup":
            if isinstance(self.setup, Exception):
                raise self.setup
            return self.setup
        raise AssertionError(f"unexpected {command}")


DLNA = {"domain": "dlna", "instance_id": "dlna", "enabled": True}
FINISH = {"type": "finish", "result": {"instance_id": "dlna"}}
RACE = ma_check.MAError("config/providers/setup", 500, "Internal server error")


def check(name, got, want):
    if got != want:
        print(f"FAIL {name}: {got!r} != {want!r}")
        sys.exit(1)
    print(f"ok   {name}")


def raises(name, stub):
    try:
        stub.ensure_provider("dlna")
    except ma_check.MAError as e:
        check(name, str(e).startswith("MA config/providers/setup"), True)
        return
    print(f"FAIL {name}: no error")
    sys.exit(1)


# MA has it already: used, not set up again
s = Stub([[DLNA]], FINISH)
check("already on", s.ensure_provider("dlna"), {"already": True, "instance_id": "dlna"})
check("already on: no setup", "config/providers/setup" in s.calls, False)
# Absent: set up through its flow
s = Stub([[]], FINISH)
check("absent: set up", s.ensure_provider("dlna"), {"instance_id": "dlna"})
# MA's start-up made it while this setup ran: the 500 and an instance that appeared meanwhile
s = Stub([[], [DLNA]], RACE)
check("made meanwhile", s.ensure_provider("dlna"), {"already": True, "instance_id": "dlna"})
# A 500 with no instance after it: an error
raises("500, nothing there", Stub([[], []], RACE))
# Another error, even with an instance after it: an error
raises("401 stays an error", Stub([[], [DLNA]], ma_check.MAError("config/providers/setup", 401, "no")))
# A disabled instance isn't one that is on
raises("disabled doesn't count", Stub([[], [dict(DLNA, enabled=False)]], RACE))
# Another domain's instance isn't this one
raises("other domain doesn't count", Stub([[], [dict(DLNA, domain="sonos")]], RACE))
print("all provider checks passed")
