"""Stand-in for the EKS Pod Identity Agent credentials endpoint."""

import http.server
import pathlib
import sys

SHARED = pathlib.Path(sys.argv[1])
PORT = int(sys.argv[2])


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        token = (SHARED / "token").read_text().strip()
        if self.path != "/v1/credentials" or self.headers.get("Authorization") != token:
            self.send_response(401)
            self.end_headers()
            return
        body = (SHARED / "credentials.json").read_bytes()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass


http.server.HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
