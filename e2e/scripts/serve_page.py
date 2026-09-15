"""Serve a directory on an ephemeral loopback port, reporting the port.

The port is written to a file rather than stdout because the caller backgrounds
this process: a backgrounded server's stdout is nobody's to read, and guessing a
fixed port would collide with whatever else the machine is running.
"""

import http.server
import os
import socketserver
import sys

directory, port_file = sys.argv[1], sys.argv[2]
os.chdir(directory)


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_args):
        """Silence per-request logging — the flow's output is the signal."""


with socketserver.TCPServer(("127.0.0.1", 0), QuietHandler) as httpd:
    with open(port_file, "w", encoding="utf-8") as handle:
        handle.write(str(httpd.server_address[1]))
    httpd.serve_forever()
