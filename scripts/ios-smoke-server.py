#!/usr/bin/env python3
"""Serves one file the way the app's audio proxy serves a stream, for the iOS Simulator run.

usage: ios-smoke-server.py <file> <port> <log>

AVPlayer probes with `Range: bytes=0-1` and insists on a proper 206, so this answers ranges with
the same headers `src-tauri/src/audioproxy.rs` sends.
"""
import http.server
import re
import sys

path, port, log_path = sys.argv[1], int(sys.argv[2]), sys.argv[3]
with open(path, "rb") as f:
    data = f.read()
log = open(log_path, "a", buffering=1)


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def respond(self, body):
        wanted = self.headers.get("Range", "")
        match = re.fullmatch(r"bytes=(\d+)-(\d*)", wanted)
        log.write(f"{self.command} range={wanted!r} ua={self.headers.get('User-Agent')!r}\n")
        if match:
            start = int(match[1])
            end = min(int(match[2]) if match[2] else len(data) - 1, len(data) - 1)
            self.send_response(206)
            self.send_header("Content-Range", f"bytes {start}-{end}/{len(data)}")
        else:
            start, end = 0, len(data) - 1
            self.send_response(200)
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Type", "audio/mp4")
        self.send_header("Content-Length", str(end - start + 1))
        self.end_headers()
        if body:
            self.wfile.write(data[start : end + 1])

    def do_GET(self):
        self.respond(True)

    def do_HEAD(self):
        self.respond(False)


http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
