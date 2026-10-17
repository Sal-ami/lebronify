"""Local preview server.

Serves two files: the page and the engine. All the work happens in the browser
now, so this process never sees an upload and never solves anything. The engine
is wasm the page loads itself.

    python tools/serve.py [port]      # default 8000

Not the shipping path. The shipping path is the same page and the same wasm on
any static host: no server logic, nothing to run.
"""

import base64
import http.server
import pathlib
import sys
import urllib.parse

ROOT = pathlib.Path(__file__).resolve().parent.parent
WASM = ROOT / "target" / "wasm32-unknown-unknown" / "release" / "lab_web.wasm"
TARGET = ROOT / "crates" / "lab-core" / "assets" / "lebron-target-256.png"

# Filled in by main() once, so each request is a byte copy rather than a re-encode.
WASM_JS = b""


def wasm_js():
    # One base64 blob wrapped in a global. The page is deliberately fetch-free,
    # so the module's bytes arrive on a script tag instead of a request.
    blob = base64.b64encode(WASM.read_bytes()).decode()
    return ('self.__LABRONIFY_WASM = "' + blob + '";\n').encode()


class Handler(http.server.BaseHTTPRequestHandler):
    def _reply(self, code, ctype, body):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = urllib.parse.urlparse(self.path).path
        if path in ("/", "/index.html"):
            self._reply(200, "text/html; charset=utf-8", (ROOT / "index.html").read_bytes())
        elif path == "/labronify.wasm.js":
            self._reply(200, "text/javascript; charset=utf-8", WASM_JS)
        elif path == "/target.png":
            self._reply(200, "image/png", TARGET.read_bytes())
        elif path.startswith("/presets/"):
            presets = (ROOT / "presets").resolve()
            file = (presets / path[len("/presets/"):]).resolve()
            if file.parent == presets and file.suffix == ".png" and file.is_file():
                self._reply(200, "image/png", file.read_bytes())
            else:
                self._reply(404, "text/plain; charset=utf-8", b"not found\n")
        else:
            self._reply(404, "text/plain; charset=utf-8", b"not found\n")

    def log_message(self, *args):
        pass


def main():
    global WASM_JS
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8000
    if not WASM.exists():
        sys.exit(
            f"wasm not found at {WASM}; build it with:\n"
            f"  cargo build -p lab-web --target wasm32-unknown-unknown --release"
        )
    WASM_JS = wasm_js()
    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)
    print(f"labronify on http://localhost:{port}/   ({len(WASM_JS) // 1024} KB engine)")
    server.serve_forever()


if __name__ == "__main__":
    main()
