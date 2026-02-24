# Holerabbit: Secure browser extension HTTP server (localhost:9876)

## Problem

`http_server.rs` sets `Access-Control-Allow-Origin: *` on all responses.
Any website the user visits can silently:
- `POST /capture` to inject nodes into the personal graph
- `GET /search?q=` to read graph contents
- `GET /status` to fingerprint that Mycelica is running

This is the last unauthenticated attack surface. The team server (port 3741)
was hardened in the security session — this is the single-user equivalent.

## Solution

Bearer token auth on all endpoints + one-click pairing flow so the user
never has to copy-paste a key between two apps on the same machine.

## Implementation

### 1. Generate extension API key on first launch

In `lib.rs` (or wherever Tauri app init runs):
- Check `settings.json` for `extension_api_key`
- If missing, generate 32 random bytes via `rand` crate, hex-encode
- Store plaintext in `settings.json` (not hashed — single-user, local only,
  if attacker has filesystem access they already have the SQLite db)
- Also store a `extension_paired: bool` flag, default `false`

### 2. Add auth middleware to http_server.rs

For every endpoint except `POST /pair`:
- Check `Authorization: Bearer <key>` header
- Compare against stored key
- Return 401 `{"error": "unauthorized"}` if missing or wrong
- Return the normal response if valid

Use the same pattern as the team server auth middleware but simpler —
no user lookup, no roles, just one key.

### 3. Add `POST /pair` endpoint (the good UX part)

This is how the extension gets the key without manual copy-paste:

```
POST /pair
Body: { "name": "Firefox Extension" }  (optional, for display)
No auth required.
```

Flow:
1. Extension calls `POST /pair`
2. Server emits a Tauri event to the frontend (or uses `tauri::api::dialog`)
3. Tauri app shows a system notification/dialog:
   "Firefox extension wants to connect to Mycelica. Allow?"
   [Allow] [Deny]
4. If allowed: server responds with `{"key": "<the-key>"}`, sets
   `extension_paired: true` in settings
5. If denied: server responds with `403 {"error": "denied"}`
6. If already paired: server responds with `409 {"error": "already_paired",
   "message": "Reset pairing in Mycelica settings to re-pair"}`

Timeout: if no user response within 30 seconds, return 408.

This is ~40 lines of Rust. The dialog is a single Tauri API call.

### 4. Update Firefox extension

In `background.js`:
- On extension install/startup, check `browser.storage.local` for `mycelica_api_key`
- If missing, call `POST http://localhost:9876/pair` with `{"name": "Firefox Extension"}`
- On success, store the returned key in `browser.storage.local`
- On 409 (already paired), show extension popup message: "Already paired.
  Reset in Mycelica settings or enter key manually."
- Add the key to all subsequent requests:
  ```js
  headers: { "Authorization": `Bearer ${key}` }
  ```
- If any request returns 401, clear stored key and re-trigger pairing

### 5. Remove CORS headers entirely

Delete all `Access-Control-Allow-Origin` headers from `http_server.rs`.

Extension background.js uses `fetch()` which is exempt from CORS
(extensions have host permissions, not subject to same-origin policy).

Content scripts DO go through CORS, but we don't use content script → 
localhost communication. If auto-capture dwell-time feature is added later,
route it through background.js via `browser.runtime.sendMessage`.

### 6. Settings panel fallback

In the Tauri React frontend settings:
- Show the extension API key (masked by default, reveal button)
- Copy button
- "Reset Pairing" button (generates new key, sets `extension_paired: false`)
- Status indicator: "Extension: Connected" / "Extension: Not paired"

This is the fallback for manual configuration or if pairing fails.
Most users should never need it.

## Files to modify

| File | Changes |
|------|---------|
| `src-tauri/src/http_server.rs` | Auth check on all endpoints, add `/pair` endpoint, remove CORS headers |
| `src-tauri/src/lib.rs` | Key generation on first launch, Tauri event handler for pairing dialog |
| `settings.json` schema | Add `extension_api_key`, `extension_paired` |
| `mycelica-firefox/background.js` | Auto-pairing on startup, Bearer header on all requests, 401 retry |
| `mycelica-firefox/popup/popup.html` | Pairing status display, manual key input fallback |
| React settings panel (new or existing) | Key display, copy, reset pairing |

## Test

```bash
# No auth → 401
curl http://localhost:9876/status
# Expected: 401 {"error": "unauthorized"}

# With auth → 200
curl -H "Authorization: Bearer <key>" http://localhost:9876/status
# Expected: 200 {"connected": true, "version": "..."}

# Pairing flow
curl -X POST http://localhost:9876/pair -H "Content-Type: application/json" -d '{"name":"test"}'
# Expected: Tauri dialog appears. Allow → 200 {"key": "..."}

# Already paired
curl -X POST http://localhost:9876/pair -H "Content-Type: application/json" -d '{"name":"test"}'
# Expected: 409 {"error": "already_paired"}

# Extension captures bookmark with key configured → node created
# Extension gets 401 → clears key → triggers re-pair
```

## Non-goals

- No hashing the key (single-user, local filesystem, same threat boundary as the db)
- No rate limiting on this server yet (team server has it planned, can port later)
- No TLS (localhost only)
- No multi-extension support (one key, one paired extension)

## Dependencies

- `rand` crate (likely already in Cargo.toml)
- `tauri::api::dialog` or Tauri event system (already available)

## Sequence

Do steps 1-2 first (key + auth). This alone fixes the vulnerability.
Steps 3-4 (pairing) are UX. Step 5 (remove CORS) is cleanup.
Step 6 (settings panel) is fallback polish.
