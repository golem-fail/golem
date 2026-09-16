"""Serve the e2e fixtures on an ephemeral loopback port, reporting the port.

Two consumers share one server, because a flow should have one thing to start
and stop: static files under the served directory (the browser portal), and the
`/__*` API routes below that the `*_http` actions exercise. The prefix keeps the
routes from ever shadowing a fixture file.

The port is written to a file rather than stdout because the caller backgrounds
this process: a backgrounded server's stdout is nobody's to read, and guessing a
fixed port would collide with whatever else the machine is running.
"""

import http.server
import json
import os
import socketserver
import sys
from urllib.parse import parse_qs, urlparse

directory, port_file = sys.argv[1], sys.argv[2]
os.chdir(directory)

API_PREFIX = "/__"
# Fixed body for the save_to / response-capture case. Short and quote-free
# values: a flow asserts on the captured body as a string.
FIXED_JSON = {"service": "golem-e2e", "version": "1", "token": "abc123"}


class Handler(http.server.SimpleHTTPRequestHandler):
    """Static files, plus the `/__*` routes the HTTP actions are tested against."""

    def log_message(self, *_args):
        """Silence per-request logging — the flow's output is the signal."""

    # SimpleHTTPRequestHandler only implements GET/HEAD; the other verbs exist
    # so every `*_http` action has something to call.
    def do_GET(self):  # noqa: N802 - name fixed by BaseHTTPRequestHandler
        if self.path.startswith(API_PREFIX):
            self.serve_api()
            return
        super().do_GET()

    def do_POST(self):  # noqa: N802
        self.serve_api()

    def do_PUT(self):  # noqa: N802
        self.serve_api()

    def do_PATCH(self):  # noqa: N802
        self.serve_api()

    def do_DELETE(self):  # noqa: N802
        self.serve_api()

    def serve_api(self):
        parsed = urlparse(self.path)
        route = parsed.path

        if not route.startswith(API_PREFIX):
            self.send_json(404, {"error": "no such route", "path": route})
            return

        if route == "/__echo":
            self.send_json(200, self.echo(parsed))
            return

        # `/__status/<code>` drives the non-2xx path: the actions fail the step
        # on any status outside 2xx.
        if route.startswith("/__status/"):
            try:
                code = int(route.rsplit("/", 1)[1])
            except ValueError:
                self.send_json(400, {"error": "status route needs a numeric code"})
                return
            self.send_json(code, {"status": code})
            return

        if route == "/__json":
            self.send_json(200, FIXED_JSON)
            return

        self.send_json(404, {"error": "no such route", "path": route})

    def echo(self, parsed):
        """Reflect the request back so a flow can assert on what it sent."""
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length).decode("utf-8") if length else ""
        return {
            "method": self.command,
            "path": parsed.path,
            "query": {k: v[0] for k, v in parse_qs(parsed.query).items()},
            # Lower-cased so an assertion doesn't depend on how the client
            # capitalised the header it sent.
            "headers": {k.lower(): v for k, v in self.headers.items()},
            "body": body,
        }

    def send_json(self, code, payload):
        encoded = json.dumps(payload, sort_keys=True).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


with socketserver.TCPServer(("127.0.0.1", 0), Handler) as httpd:
    with open(port_file, "w", encoding="utf-8") as handle:
        handle.write(str(httpd.server_address[1]))
    httpd.serve_forever()
