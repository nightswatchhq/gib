#!/usr/bin/env python3
"""gib topology-adapter — network-subgraph envelope shim.

**Why this exists.** The gateway's `trusted_indexers` expects an *indexer-service*
response envelope:

    {"graphQLResponse": "<stringified inner GraphQL JSON>", "attestation": null}

A fresh gib deployment has no indexer of its own, and post-Horizon every real
indexer (including indexer.upgrade.thegraph.com) gates the network subgraph
behind a TAP receipt — there is no free, keyless source of Arbitrum topology
(verified: unauthenticated queries get `402 "No Tap receipt was found"`). The one
thing that serves the network subgraph without an escrow/whitelist relationship
is The Graph's decentralised gateway (gateway.thegraph.com) with a **read-only**
API key. But it returns bare `{"data": ...}`, not the envelope.

This adapter bridges that gap: it accepts the gateway's GraphQL POST, forwards it
to the keyed decentralised-gateway URL (with a browser UA — the WAF 403s
`python-urllib`), and re-wraps the response into the envelope. The key stays in
this adapter's env only; it signs nothing and holds no funds. Attestation is
`null` — the gateway verifies attestations only on the *paid* path, never on the
trusted-indexer/network-subgraph (Free) path.

This is the **pragmatic default** topology source while no free public
network-subgraph endpoint exists. The two sovereign alternatives — self-indexing
the network subgraph, or a trusted arrangement with a cooperating indexer that
free-serves you — need no key and no adapter; see docs/06-topology.md.

Config (env):
  STUDIO_API_KEY        read-only decentralised-gateway (Studio) key. REQUIRED
                        unless UPSTREAM_URL is set.
  NETWORK_SUBGRAPH_ID   subgraph id to query (default: Graph Network Arbitrum).
  STUDIO_GATEWAY        gateway base (default https://gateway.thegraph.com/api).
  UPSTREAM_URL          full keyed URL; overrides the three above if set.
  ADAPTER_LISTEN        host:port to bind (default 0.0.0.0:7601).
  USER_AGENT            browser UA to dodge the upstream WAF.

Stdlib only — no third-party deps.
"""
import json
import os
import sys
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# Graph Network Arbitrum (decentralised-gateway subgraph id).
DEFAULT_SUBGRAPH_ID = "DZz4kDTdmzWLWsV373w2bSmoar3umKKH9y82SUKr5qmp"

STUDIO_API_KEY = os.environ.get("STUDIO_API_KEY", "").strip()
NETWORK_SUBGRAPH_ID = os.environ.get("NETWORK_SUBGRAPH_ID", DEFAULT_SUBGRAPH_ID).strip()
STUDIO_GATEWAY = os.environ.get("STUDIO_GATEWAY", "https://gateway.thegraph.com/api").strip().rstrip("/")

UPSTREAM_URL = os.environ.get("UPSTREAM_URL", "").strip()
if not UPSTREAM_URL and STUDIO_API_KEY:
    UPSTREAM_URL = f"{STUDIO_GATEWAY}/{STUDIO_API_KEY}/subgraphs/id/{NETWORK_SUBGRAPH_ID}"

USER_AGENT = os.environ.get(
    "USER_AGENT",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/125.0 Safari/537.36",
)
_listen = os.environ.get("ADAPTER_LISTEN", "0.0.0.0:7601")
HOST, _, PORT = _listen.partition(":")
PORT = int(PORT or "7601")


def _redacted_upstream():
    """Show host + subgraph tail, never the key."""
    try:
        pre, _, post = UPSTREAM_URL.partition("/api/")
        tail = post.split("/subgraphs/id/")[-1][-8:] if "/subgraphs/id/" in post else "?"
        return f"{pre}/api/<KEY>/subgraphs/id/…{tail}"
    except Exception:
        return "<unparseable>"


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):  # quiet default logging
        pass

    def _send_json(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/health":
            # Healthy only once configured — makes compose `service_healthy`
            # meaningful: the gateway won't boot into a broken topology loop.
            if UPSTREAM_URL:
                self._send_json(200, {"ok": True, "upstream": _redacted_upstream()})
            else:
                self._send_json(503, {"ok": False, "error": "STUDIO_API_KEY not set"})
        else:
            self._send_json(404, {"error": "POST a GraphQL query; GET /health"})

    def do_POST(self):
        if not UPSTREAM_URL:
            self._send_json(200, {"error": "adapter not configured: set STUDIO_API_KEY"})
            return
        length = int(self.headers.get("Content-Length", "0") or "0")
        body = self.rfile.read(length) if length else b""
        # Forward the gateway's GraphQL body verbatim to the keyed upstream.
        req = urllib.request.Request(
            UPSTREAM_URL,
            data=body,
            method="POST",
            headers={
                "Content-Type": "application/json",
                "User-Agent": USER_AGENT,
                "Accept": "application/json",
            },
        )
        try:
            with urllib.request.urlopen(req, timeout=30) as resp:
                raw = resp.read().decode("utf-8", "replace")
        except urllib.error.HTTPError as e:
            detail = e.read().decode("utf-8", "replace")[:200]
            self._send_json(200, {"error": f"upstream HTTP {e.code}: {detail}"})
            return
        except Exception as e:
            self._send_json(200, {"error": f"upstream error: {e}"})
            return
        # raw is `{"data": ...}`. Re-wrap into the indexer-service envelope the
        # gateway parses. graphQLResponse is a STRING; attestation is null.
        self._send_json(200, {"graphQLResponse": raw, "attestation": None})


def main():
    if not UPSTREAM_URL:
        sys.stderr.write(
            "gib topology-adapter: STUDIO_API_KEY not set — /health will report 503 "
            "until configured (see .env.example TOPOLOGY_STUDIO_KEY).\n"
        )
    srv = ThreadingHTTPServer((HOST, PORT), Handler)
    sys.stderr.write(
        f"gib topology-adapter listening on {HOST}:{PORT} -> "
        f"{_redacted_upstream() if UPSTREAM_URL else '<unconfigured>'}\n"
    )
    srv.serve_forever()


if __name__ == "__main__":
    main()
