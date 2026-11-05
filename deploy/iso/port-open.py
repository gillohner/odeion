#!/usr/bin/env python3
"""CI (test-qemu.sh): whether something in the VM listens behind a QEMU port forward.

    port-open.py <host> <port>

QEMU's user network accepts the connection itself and then connects into the VM: when nothing
listens there it closes at once (EOF). A TLS server instead waits for the client to speak first.
Exit 0 when the connection stays open for 3 s (or the server sends anything), 1 otherwise."""
import socket
import sys

host, port = sys.argv[1], int(sys.argv[2])
try:
    s = socket.create_connection((host, port), timeout=5)
except OSError:
    sys.exit(1)
s.settimeout(3)
try:
    data = s.recv(1)
except socket.timeout:
    sys.exit(0)  # still open, waiting for us: a listener
except OSError:
    sys.exit(1)
sys.exit(0 if data else 1)
