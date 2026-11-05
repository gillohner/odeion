#!/usr/bin/env python3
"""CI (test-qemu.sh): answer one installer question on the VM's serial console like a person would.

    answer-console.py <socket> <text on screen> [keys]

Waits (up to 40 min) until <text> shows on the serial console, then sends the keys (default:
Enter, which takes the highlighted default) and keeps reading until the VM goes away, so the
console never stalls. Exit 0 when it answered, 1 when the text never came."""
import re
import socket
import sys
import time

ANSI = re.compile(rb"\x1b(\[[0-9;?]*[ -/]*[@-~]|[()][0-9A-Za-z]|.)")


def plain(b):
    """The screen's text: terminal escapes and all whitespace taken out (they fall between words)."""
    return re.sub(rb"\s+", b"", ANSI.sub(b"", b))

sock_path, text = sys.argv[1], plain(sys.argv[2].encode())
keys = (sys.argv[3] if len(sys.argv) > 3 else "\r").encode()
for _ in range(60):
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.connect(sock_path)
        break
    except OSError:
        time.sleep(1)
else:
    sys.exit(1)
s.settimeout(5)
seen, answered, end = b"", False, time.time() + 40 * 60
while True:
    try:
        chunk = s.recv(65536)
    except socket.timeout:
        chunk = None
    except OSError:
        break
    if chunk == b"":
        break
    if chunk and not answered:
        seen = (seen + chunk)[-20000:]
        if text in plain(seen):
            time.sleep(2)  # let the dialog finish drawing
            s.sendall(keys)
            answered = True
            print(f"answered {sys.argv[2]!r}", flush=True)
    if not answered and time.time() > end:
        break
sys.exit(0 if answered else 1)
