#!/usr/bin/env python3
"""CI (test-qemu.sh): log in on the test VM's serial console and run commands, printing what they show.

    vm-console.py <socket> <user> <password> <command>...

The VM's serial port is a unix socket (QEMU -chardev socket). Each command runs with sudo; the
output ends at a marker. Only for the CI test VM, whose password is the test's own."""
import socket
import sys
import time

sock_path, user, password, *commands = sys.argv[1:]
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(sock_path)
s.settimeout(1)


def read_until(token, limit):
    buf, end = b"", time.time() + limit
    while time.time() < end:
        try:
            chunk = s.recv(65536)
        except socket.timeout:
            chunk = b""
        if chunk:
            buf += chunk
            if token.encode() in buf:
                return buf.decode(errors="replace")
    return buf.decode(errors="replace")


def send(line):
    s.sendall(line.encode() + b"\n")


send("")
if "login:" not in read_until("login:", 60):
    send("")
    read_until("login:", 30)
send(user)
read_until("Password:", 30)
send(password)
read_until("$ ", 60)
for i, cmd in enumerate(commands):
    marker = f"__done_{i}__"
    # The quotes keep the terminal's echo of this line from matching the marker
    send(f"echo {password} | sudo -S -p '' sh -c {cmd!r} 2>&1; echo __done_''{i}__")
    out = read_until(marker + "\r\n", 120)
    print(f"\n### {cmd}\n{out}", flush=True)
send("exit")
