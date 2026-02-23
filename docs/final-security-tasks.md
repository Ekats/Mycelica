# Final Security Tasks — Implementation Prompts

## Task 1: TLS via Caddy (Deployment Config)

This is a deployment script, not application code. Create a setup script
that can be run on the team server.

```
Create a deployment script for mycelica-server behind Caddy reverse proxy.

File: scripts/deploy-server.sh

The script should:

1. Check if Caddy is installed, print install instructions if not:
   sudo apt install -y debian-keyring debian-archive-keyring apt-transport-https
   curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | sudo gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
   curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | sudo tee /etc/apt/sources.list.d/caddy-stable.list
   sudo apt update && sudo apt install caddy

2. Generate a Caddyfile at /etc/caddy/Caddyfile:
   - If a domain is provided as $1, use it with automatic Let's Encrypt
   - If no domain, use :3743 with internal TLS (self-signed)
   - Reverse proxy to 127.0.0.1:3741

   # With domain:
   mycelica.example.ee {
       reverse_proxy 127.0.0.1:3741
   }

   # Without domain (LAN only, self-signed):
   :3743 {
       tls internal
       reverse_proxy 127.0.0.1:3741
   }

3. Create a systemd service file for mycelica-server at
   /etc/systemd/system/mycelica-server.service:

   [Unit]
   Description=Mycelica Team Server
   After=network.target

   [Service]
   Type=simple
   User=mycelica
   ExecStart=/usr/local/bin/mycelica-server --db /var/lib/mycelica/team.db --bind 127.0.0.1:3741
   Restart=always
   RestartSec=5
   Environment=RUST_LOG=info

   [Install]
   WantedBy=multi-user.target

4. Print a summary of what was configured and next steps:
   - Create the mycelica user: sudo useradd -r -s /bin/false mycelica
   - Copy binary: sudo cp mycelica-server /usr/local/bin/
   - Init DB: sudo -u mycelica mycelica-server --db /var/lib/mycelica/team.db admin create-key <name> --role admin
   - Start services: sudo systemctl enable --now mycelica-server caddy
   - Test: curl https://localhost:3743/health (self-signed) or curl https://mycelica.example.ee/health

The script should NOT auto-run anything dangerous. It generates configs
and prints instructions. The operator (Ekats) runs the actual commands.

Make it idempotent — safe to run multiple times.

Also create scripts/create-member-key.sh:
  #!/bin/bash
  # Usage: ./create-member-key.sh <username> [admin|editor]
  ROLE=${2:-editor}
  mycelica-server --db /var/lib/mycelica/team.db admin create-key "$1" --role "$ROLE"

Build: N/A (shell scripts only)
Test: shellcheck scripts/deploy-server.sh
```

---

## Task 2: Team React Settings Panel — API Key Input

```
Add API key configuration to the team app's settings UI.

First, find where the team frontend lives:
- Check src-team/ for React/TypeScript files
- Check tauri.team.conf.json for the frontend dev/build paths
- Check package.json or any npm workspace config
- If there's a shared src/ with team-specific components, find them
- Run: find /home/ekats/Repos/Mycelica -name "*.tsx" -path "*team*" -o -name "*.tsx" -path "*Team*" -o -name "*.tsx" -path "*Settings*" | head -20

Once you've found the team frontend:

1. Find or create a Settings component. Look for existing settings
   patterns in the personal app (src/components/Settings* or similar)
   as a reference.

2. Add a "Server Connection" section with:
   - Server URL input (pre-filled from config, e.g. http://localhost:3741)
   - API Key input (type="password", with show/hide toggle button)
   - Username display (read-only, shown after successful auth test)
   - "Test Connection" button that:
     a. Hits GET /health on the server URL (verifies reachability)
     b. If API key is set, hits POST /nodes with a dry-run or
        just checks the auth by looking at the response code
        (a 401 means bad key, anything else means it works)
     c. Shows green "Connected as <username>" or red error message
   - "Save" button that persists to team config

3. The save action should invoke the existing Tauri command:
   Check src-tauri/src/commands/team.rs for team_save_settings or similar.
   The TeamConfig struct already has api_key: Option<String> field.

4. Visual design:
   - Match the personal app's settings panel style
   - API key field should have a copy button (for sharing with the user
     who generated it, not for the key itself)
   - Show "Read-only mode" indicator when no API key is configured
   - Show "Authenticated" indicator when key is present and tested

5. On app startup, if api_key is set in config, automatically test
   the connection in the background and show status in the header/toolbar.

Tauri commands to use:
- invoke('team_save_settings', { config: { server_url, api_key, ... } })
- invoke('team_refresh') — to test connectivity after saving

Check: src-tauri/src/commands/team.rs for available commands
Check: The personal app's settings for UI patterns to follow
Check: How the team app currently displays server connection status

Build: npm run build (from team frontend directory)
Build: cargo +nightly build --features mcp
Test: Open team app, enter server URL, enter API key, click test, verify
      "Connected as <username>" appears. Save, restart app, verify key persists.
```

---

## Task 3: Browser Extension CORS Fix — Pairing Flow

```
Fix the Access-Control-Allow-Origin: * vulnerability on the personal app's
built-in HTTP server (localhost:9876) by adding a pairing-based auth flow.

This is NOT related to the team server. This is the personal Tauri app's
browser extension integration server in src-tauri/src/http_server.rs.

### Step 1: Generate extension API key on first launch

In src-tauri/src/settings.rs (or wherever Settings struct lives):
- Add field: extension_api_key: Option<String>
- On app startup, if extension_api_key is None, generate a 32-byte
  random key using rand::thread_rng() and base64-encode it
- Persist to settings.json immediately
- This key never changes unless the user explicitly regenerates it

Check: How settings are loaded/saved. The field should survive app restarts.
Check: rand crate is already a dependency (used for API key generation
in the team server code). If not, add it.

### Step 2: Add POST /pair endpoint to http_server.rs

New endpoint that does NOT require auth:
- Rate limit: 1 attempt per 30 seconds (simple timestamp check, no crate needed)
- When hit, trigger a Tauri dialog/notification:
  "Firefox extension wants to connect to Mycelica. Allow?"
  with Yes/No buttons
- If Yes: return 200 {"key": "<extension_api_key>"}
- If No: return 403 {"error": "rejected"}
- If rate limited: return 429 {"error": "try_again_later"}

For the Tauri dialog, check how to trigger a dialog from a non-Tauri thread.
The HTTP server runs in a background thread (tiny_http). Options:
a. Use tauri::api::dialog (if accessible from the http server thread)
b. Use a channel: http server sends a pairing request through an mpsc channel,
   Tauri main thread receives it, shows dialog, sends response back
c. Use std::sync::mpsc or tokio::sync::oneshot for the response

Option (b) is cleanest. The http_server already has access to some shared
state — check what's passed in when the server thread is spawned.

If triggering a Tauri dialog is too complex for the current architecture,
fall back to: print the key to stdout on first launch and require manual
copy-paste. The /pair endpoint would then just always return 403 with
a message telling the user to check the settings panel. Less elegant but
still secure.

### Step 3: Auth middleware on all OTHER endpoints

In http_server.rs, for every handler (GET /status, GET /search, POST /capture):
- Check for Authorization: Bearer <key> header
- Compare against extension_api_key from settings
- If missing or wrong: return 401 {"error": "unauthorized"}
- If correct: proceed to handler

The comparison should be constant-time to prevent timing attacks,
but for a localhost-only server this is not critical. A simple == is fine.

### Step 4: Remove CORS headers entirely

Delete all Access-Control-Allow-Origin headers from http_server.rs.

Firefox extension background scripts (using browser.fetch or fetch() from
background.js) are NOT subject to same-origin policy. They never needed
CORS headers. The * was unnecessary from day one.

Content scripts DO go through CORS, but the extension currently only
makes requests from background.js. If auto-capture ever moves to a
content script, it would need to message background.js which then
makes the fetch. This is the correct architecture anyway.

### Step 5: Update Firefox extension

Check where the extension source lives:
- Look for manifest.json, background.js, popup.html
- Might be in a separate repo (mycelica-firefox or similar)
- Or under src-tauri/extension/ or similar

Changes to the extension:

1. On first install (or if no key in storage):
   - Call POST http://localhost:9876/pair
   - If 200: store key in browser.storage.local
   - If 403 or error: show "Please open Mycelica and approve the connection"
     in the extension popup
   - Add a "Retry pairing" button in the popup

2. On every request to localhost:9876:
   - Read key from browser.storage.local
   - Add Authorization: Bearer <key> header
   - If 401 response: clear stored key, trigger re-pairing

3. Extension popup should show connection status:
   - Green: "Connected to Mycelica"
   - Red: "Not connected — click to pair"
   - Yellow: "Mycelica app not running" (when localhost:9876 is unreachable)

### Step 6: Settings panel in Tauri app

Add to the personal app's settings panel (not team app):
- "Browser Extension" section
- Show extension_api_key (masked by default, with show/copy toggle)
- "Regenerate Key" button (generates new key, invalidates old one —
  extension will get 401 and need to re-pair)
- Connection status if possible (track last successful request timestamp)

Check: Where the personal app's settings UI lives
Check: src-tauri/src/http_server.rs for current implementation
Check: Firefox extension source location

Build: cargo +nightly build --features mcp
Build: Firefox extension (web-ext build or however it's currently packaged)
Test: curl http://localhost:9876/status → 401
Test: curl -H "Authorization: Bearer <key>" http://localhost:9876/status → 200
Test: Open Firefox, extension pairs automatically, captures work
Test: Regenerate key in settings → extension gets 401 → re-pairs on click
```

---

## Execution Order

1. Task 2 first (Team GUI settings) — this is blocking team server UX.
   Without it, users have to hand-edit JSON config files.
2. Task 1 (Caddy deployment script) — needed on deploy day.
3. Task 3 (Extension CORS) — not blocking anything, do after deployment.
