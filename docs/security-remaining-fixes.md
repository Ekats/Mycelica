# Remaining Security Fixes — Implementation Prompts

## Fix 1: Rate Limiting (Step 6)

```
Add per-IP rate limiting to mycelica-server write endpoints.

Add `tower_governor` to Cargo.toml dependencies.

In server.rs:
- Apply governor rate limiting middleware to POST/PATCH/DELETE routes only
- 60 requests per minute per IP address
- GET endpoints are NOT rate limited (public reads should be fast)
- Return 429 Too Many Requests with a JSON body: {"error": "rate_limited", "retry_after_seconds": N}
- Include Retry-After header in 429 responses

When --no-auth is active, still apply rate limiting. Rate limiting protects
server resources regardless of auth model.

Check: tower_governor crate docs for axum integration pattern.
Check: existing middleware stack in server.rs — rate limit layer should be
OUTSIDE the auth layer (rate limit fires before auth check, prevents
unauthenticated flood from even hitting the auth lookup).

Test: Start server, run `for i in $(seq 1 70); do curl -s -o /dev/null -w "%{http_code}\n" -X POST -H "Authorization: Bearer <key>" -H "Content-Type: application/json" -d '{"title":"test"}' http://localhost:3741/nodes; done`
Expect: first 60 return 201, remaining return 429.

Build: cargo +nightly build --features mcp --bin mycelica-server
```

---

## Fix 2: Input Validation (Step 7)

```
Add input size limits to mycelica-server write handlers.

No new crates needed. Manual validation in handler functions before
touching the database.

Limits:
- Node title: max 2000 chars. Return 400 if exceeded.
- Node content: max 1MB (1_048_576 bytes). Return 400 if exceeded.
- Node tags: max 50 tags, each max 500 chars. Return 400 if exceeded.
- Edge reason: max 2000 chars. Return 400 if exceeded.
- Author field: max 100 chars (defense in depth — server sets this from
  API key, but validate anyway in case --no-auth mode passes client author).

Also add axum body size limit:
- Add `tower_http::limit::RequestBodyLimitLayer` to the middleware stack
- Set global limit to 2MB (covers JSON overhead around 1MB content)
- This catches oversized requests before they even reach handlers

Error format (consistent with existing AppError):
{
  "error": "validation_error",
  "detail": "title exceeds maximum length of 2000 characters"
}

Create a validation helper function:
fn validate_node_input(title: &str, content: Option<&str>, tags: Option<&[String]>) -> Result<(), AppError>
fn validate_edge_input(reason: Option<&str>) -> Result<(), AppError>

Call these at the top of create_node_handler, patch_node_handler,
create_edge_handler, patch_edge_handler — before any database operations.

Check: src-tauri/src/bin/server.rs for handler signatures and AppError type.
Check: tower-http already in Cargo.toml — verify "limit" feature is enabled,
add it if not.

Test: curl -X POST -H "Authorization: Bearer <key>" -H "Content-Type: application/json" -d '{"title":"'$(python3 -c "print('x'*2001)")'" }' http://localhost:3741/nodes
Expect: 400 with validation_error message.

Build: cargo +nightly build --features mcp --bin mycelica-server
```

---

## Fix 3: Team GUI API Key Settings Panel

```
Add API key configuration to the team GUI settings.

Context: The Rust backend (remote_client.rs, team.rs) already supports
api_key in TeamConfig and attaches Authorization headers to writes.
What's missing is a way for the user to enter the key in the GUI.

Check where the team frontend lives. Look for:
- src-team/ directory
- Any .tsx/.ts files with "team" or "Team" in the name under src/
- tauri.team.conf.json for the team app's frontend config
- The team app's entry point and settings/config components

If src-team/ exists with React components:
1. Find or create a Settings component/panel
2. Add an "API Key" text input field (type="password" for masking)
3. Add a "Save" button that writes to the team config
4. Add a "Show/Hide" toggle for the key field
5. On save, call a Tauri command that updates TeamConfig.api_key
   and recreates the RemoteClient with the new key
6. Show connection status: "Authenticated as <username>" or
   "No API key configured — read-only mode" based on whether
   the key is set

If no team React frontend exists yet:
1. Create minimal src-team/ with a settings page
2. Include: server URL input, API key input, connection test button
3. The connection test hits GET /health on the server URL to verify
   reachability, then tries a dummy authenticated request to verify
   the key works
4. Store config via Tauri invoke to team_save_settings command

Tauri command needed (check if it exists):
#[tauri::command]
async fn team_save_settings(state: State<TeamState>, config: TeamConfig) -> Result<(), String>

This likely already exists in src-tauri/src/commands/team.rs — check
the current implementation and verify it persists api_key to disk.

Check: src-tauri/src/commands/team.rs for team_save_settings
Check: How the personal app's settings panel works (src/components/Settings*)
       as a pattern to follow for the team settings

Build: npm run build (or whatever the team frontend uses)
Build: cargo +nightly build --features mcp
```

---

## Fix 4: TLS via Caddy (Deployment)

```
This is NOT a code change. This is deployment configuration for the
team server.

Option A — Caddy reverse proxy (recommended):

Install Caddy on the team server:
  sudo apt install -y debian-keyring debian-archive-keyring apt-transport-https
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | sudo gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | sudo tee /etc/apt/sources.list.d/caddy-stable.list
  sudo apt update
  sudo apt install caddy

Create /etc/caddy/Caddyfile:

  mycelica.local {
    reverse_proxy 127.0.0.1:3741
    tls internal
  }

  # Or if you have a real domain + Let's Encrypt:
  # mycelica.example.ee {
  #   reverse_proxy 127.0.0.1:3741
  # }

Start Caddy:
  sudo systemctl enable --now caddy

With this setup:
- Caddy handles TLS termination (self-signed or Let's Encrypt)
- mycelica-server stays on 127.0.0.1:3741 (not exposed directly)
- Users connect to https://mycelica.local or https://mycelica.example.ee
- Caddy auto-renews Let's Encrypt certs

For local network without a domain, use Caddy's internal TLS (self-signed).
Users will see a browser warning on first visit — acceptable for a local network.

Option B — No TLS:

If the network is a trusted LAN and you don't want the complexity:
- Run mycelica-server with --bind 0.0.0.0:3741
- Accept that traffic is plaintext on the local network
- API keys are sent in cleartext over the wire
- Acceptable risk if: LAN is switched (not WiFi), or WiFi uses WPA2/3

For a 2-day deadline, Option B is honest and fine. TLS is important but
not blocking. Add it after the demo if needed.
```

---

## Fix 5: Browser Extension CORS (separate task, not team server)

```
Fix CORS vulnerability on Mycelica's built-in HTTP server (localhost:9876).

NOT part of the team server deployment. This is for the personal app's
browser extension integration.

Current state: Access-Control-Allow-Origin: * in http_server.rs
Problem: Any website can POST /capture or GET /search on localhost:9876

Solution: Pairing flow.

1. On first Tauri app launch (or if key missing), generate a 32-byte
   random key via rand crate, store in settings.json as extension_api_key

2. Add POST /pair endpoint:
   - No auth required
   - Triggers a Tauri dialog: "Firefox extension wants to connect. Allow?"
   - If user clicks Yes: return {"key": "<the extension_api_key>"} once
   - If user clicks No: return 403
   - Rate limit: only allow 1 pair attempt per 30 seconds

3. Auth middleware on all OTHER endpoints:
   - Require Authorization: Bearer <key> matching extension_api_key
   - Return 401 without valid key

4. Remove ALL CORS headers. Firefox extension background scripts
   (browser.fetch) bypass same-origin policy entirely. CORS was never
   needed — the * was a shortcut that should never have shipped.

5. Update Firefox extension:
   - On first install or if key missing: call POST /pair on localhost:9876
   - Store returned key in browser.storage.local
   - Attach Authorization: Bearer <key> to every request

6. Settings panel (Tauri app):
   - Show extension_api_key (read-only, copy button) as manual fallback
   - Show "Extension connected" / "Not connected" status

Check: src-tauri/src/http_server.rs for current CORS and handler code
Check: Firefox extension source for how it makes requests to localhost
Check: Tauri dialog API for confirmation prompts

Test: curl http://localhost:9876/status returns 401
Test: curl -H "Authorization: Bearer <key>" http://localhost:9876/status returns 200
Test: Firefox extension still captures after pairing
```

---

## Execution Order

For the team server deployment:
1. Fix 1 (rate limiting) — 30 min
2. Fix 2 (input validation) — 30 min
3. Fix 3 (team GUI key input) — depends on whether src-team/ exists
4. Fix 4 (TLS) — deployment day, 15 min with Caddy

Not blocking deployment:
5. Fix 5 (extension CORS) — separate PR, do after demo
