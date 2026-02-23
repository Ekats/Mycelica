# Security Hardening Session Transcript

Session: 2026-02-23/24
Plan: noble-stirring-waffle

---

    Read /home/ekats/Repos/Mycelica/docs/security-investigation-prompt.md
I've read the document thoroughly. Here's my understanding:

**What it is**: A structured security investigation prompt for assessing Mycelica's readiness for team server deployment. The threat model shifts from "4 trusted people on private Tailscale" to "semi-trusted users on a shared network" — a fundamentally different security posture.

**6 investigation tracks** (designed for parallel agents):
1. **Auth & Security State** — Does any auth exist? What endpoints are unprotected?
2. **Database & Input Handling** — SQL injection risks, input validation, sanitization
3. **Unwrap Panics** — 50+ `.unwrap()` calls that become DoS vectors in a multi-user server
4. **Network Exposure & CORS** — What binds where, what's reachable
5. **Dependency Audit** — Vulnerable crates, attack surface
6. **Team Mode Status** — What's actually built vs. still in docs

**Synthesis deliverable**: A single report covering current posture, gap analysis, implementation recommendations (with crate choices and complexity estimates), a sequenced implementation order, and open questions.

**Key constraints**: Investigation only, no implementation. The document is very explicit about this.

The investigation is well-designed — it targets the right attack surfaces for the threat model change. The unwrap/panic investigation (Agent 3) is particularly astute since mutex poisoning in a shared server is a real kill-the-whole-thing risk.

    Ask: Do you want me to execute this security investigation now? I'd run all 6 tracks 
Launching the investigation. I'll run 3 explore agents now (Auth, Database, Unwraps), then the remaining 3.

    Agent(Explore): Security: Auth & Security State
    Agent(Explore): Security: DB Schema & Input
    Agent(Explore): Security: Unwraps & Stability
First 3 tracks complete. Launching the remaining 3 (Network, Dependencies, Team Mode).

    Agent(Explore): Security: Network & CORS
    Agent(Explore): Security: Dependency Audit
    Agent(Explore): Security: Team Mode Status
All 6 tracks complete. Let me read a couple of critical files to validate key findings before writing the synthesis.

    Read /home/ekats/Repos/Mycelica/src-tauri/src/bin/server.rs
    Read /home/ekats/Repos/Mycelica/src-tauri/src/bin/server.rs
    Read /home/ekats/Repos/Mycelica/src-tauri/src/db/schema.rs
Findings validated. Now writing the synthesis report as the plan file.

    Write noble-stirring-waffle.md
Investigation complete. The plan file has the full synthesis. Before I finalize, I need your input on the two most impactful design decisions:

    Ask: API keys or user accounts? API keys are simpler (generate via CLI, distribute to
    Ask: What role granularity do you want?
Clear decisions. Let me update the plan with these specifics.

    Read /home/ekats/.claude/plans/noble-stirring-waffle.md
    Edit noble-stirring-waffle.md: "## 3. Implementation Recommendations  ### Layer 1: Auth Midd..."
    Edit noble-stirring-waffle.md: "### Step 2: Auth Middleware + API Keys - Add `api_keys` tabl..."
    Edit noble-stirring-waffle.md: "## 5. Open Questions  1. **API keys vs. user accounts?** API..."
Now update the verification section to match the public-reads model:

    Edit noble-stirring-waffle.md: "## Verification  1. `cargo +nightly build --features mcp` — ..."
Plan is updated with your decisions:

- **Public reads, authenticated writes** — GET endpoints open, POST/PATCH/DELETE require bearer token
- **admin/editor roles** — viewer is just "no key"
- **Server-side author enforcement** — `author` set from API key, not client
- **Editor ownership** — can only delete/edit own nodes, admin can touch anything

The plan covers 7 implementation steps in priority order, starting with mutex safety (foundational), then auth, then CORS/bind/rate-limiting/validation/TLS. All using existing crates where possible (sha2, tower-http).

    ExitPlanMode
Clear on all fronts. Let me update the plan with these final decisions and the GUI client requirement.

    Read /home/ekats/.claude/plans/noble-stirring-waffle.md
    Edit noble-stirring-waffle.md: "## 5. Open Questions (resolved + remaining)  **Resolved:** -..."
Now update the implementation steps and key files to reflect the GUI client requirement, the `--no-auth` flag, and the exclusion of `http_server.rs`:

    Edit noble-stirring-waffle.md: "### Layer 3: CORS Configuration (HIGH)  **Approach:** Use `t..."
    Edit noble-stirring-waffle.md: "### Step 2: Auth Middleware + API Keys - Add `api_keys` tabl..."
    Edit noble-stirring-waffle.md: "### Step 5: Rate Limiting - Add per-IP rate limiting on writ..."
    Edit noble-stirring-waffle.md: "## Key Files to Modify  | File | Changes | |------|---------..."
    Edit noble-stirring-waffle.md: "## Verification  1. `cargo +nightly build --features mcp` — ..."
Plan fully updated with all your decisions. Summary of what changed:

- **`--no-auth` flag** added to Step 2 (default: auth on, Kyberpunk uses `--no-auth`)
- **Mutex fix:** schema.rs only, follow-up for the rest
- **Browser extension CORS:** removed from scope entirely, `http_server.rs` not touched
- **`/snapshot`:** stays public
- **Step 3 added:** Team GUI client update (API key in settings, `remote_client.rs` attaches auth headers to writes)
- Steps renumbered (now 8 total)

    ExitPlanMode
Understood — I'll implement directly, no delegation. Exiting plan mode now.

    ExitPlanMode
Plan approved. Let me create tasks and start implementing.

    TaskCreate: Step 1: Mutex safety — add conn() helper, replace 233 unwraps in schema.rs
    TaskCreate: Step 2: Auth middleware + API keys + --no-auth flag in server.rs
    TaskCreate: Step 3: Team GUI client — API key in settings + remote_client.rs auth headers
    TaskCreate: Step 4: CORS policy on team server
    TaskCreate: Step 5: Change default bind from 0.0.0.0 to 127.0.0.1
    Task #1 -> in_progress
Starting with Step 1: Mutex safety. Let me read the full Database struct and understand the pattern.

    Read /home/ekats/Repos/Mycelica/src-tauri/src/db/schema.rs
I need to add a `conn()` helper and replace 233 `self.conn.lock().unwrap()` calls. Let me first check the exact pattern and count.

    Grep "self\.conn\.lock\(\)\.unwrap\(\)" in schema.rs
231 occurrences. Let me add the `conn()` helper method and then do the mass replacement.

    Edit schema.rs: "use std::sync::Mutex;..."
Now add the `conn()` helper right after `raw_conn()`:

    Edit schema.rs: "    /// Get raw access to the connection mutex (for custom s..."
Now do the mass replacement — change all `self.conn.lock().unwrap()` to `self.conn()?`:

    $ sed -i 's/self\.conn\.lock()\.unwrap()/self.conn()?/g' src-tauri/src/db/schema.rs
Verify the replacement worked and no unwraps remain:

    Grep "self\.conn\.lock\(\)\.unwrap\(\)" in schema.rs
    Grep "self\.conn\(\)\?" in schema.rs
All 231 replaced, 0 remaining. Now let me verify it compiles:

    $ cd /home/ekats/Repos/Mycelica/src-tauri && cargo +nightly build --features mcp,cuda 2>&1 | tail -20
No CUDA driver on this machine. Build without cuda:

    $ cargo +nightly build --features mcp 2>&1 | tail -20
Compile errors. Let me see the actual errors (not just warnings):

    $ cargo +nightly build --features mcp 2>&1 | grep "^error"
Two methods don't return `Result`. Let me find them:

    $ cargo +nightly build --features mcp 2>&1 | grep -B5 "E0277"
Two methods that don't return `Result`: `is_descendant_of` returns `bool`. Let me find both and fix them.

    $ cargo +nightly build --features mcp 2>&1 | grep "this function should return"
    $ cargo +nightly build --features mcp 2>&1 | grep -E "(cannot use|this function should)" 
Two functions: one returns `bool`, one returns `HashSet<String>`. Let me read them:

    Read /home/ekats/Repos/Mycelica/src-tauri/src/db/schema.rs
    Grep "-> HashSet<String>" in schema.rs
    Grep "HashSet" in schema.rs
    Read /home/ekats/Repos/Mycelica/src-tauri/src/db/schema.rs
Both functions don't return `Result`, so `?` can't be used. For these, I'll use `unwrap_or_else` to recover from poisoning (since they can't propagate errors):

    Edit schema.rs: "    pub fn is_descendant_of(&self, node_id: &str, ancestor_i..."
    Edit schema.rs: "    pub fn get_protected_node_ids(&self) -> std::collections..."
Now rebuild:

    $ cargo +nightly build --features mcp 2>&1 | grep "^error" || echo "BUILD SUCCESS"
Compiles clean. Quick smoke test:

    $ cargo +nightly run --features mcp --bin mycelica-cli -- db stats 2>&1 | head -20
    $ cargo +nightly run --features mcp --bin mycelica-cli -- db stats 2>&1 | grep -v "^warning"
CLI works perfectly. 1185 items, 7703 edges, 489 categories. Step 1 complete.

    Task #1 -> completed
    Task #2 -> in_progress
Now Step 2: Auth middleware + API keys. Let me read the key files I need to modify.

    Read /home/ekats/Repos/Mycelica/src-tauri/src/db/models.rs
    Read /home/ekats/Repos/Mycelica/src-tauri/src/bin/server.rs
Good, I have the full server.rs. Now let me implement Step 2 piece by piece.

**First: Add ApiKey model to models.rs**

    Read /home/ekats/Repos/Mycelica/src-tauri/src/db/models.rs
    Grep "^pub struct" in models.rs
Let me add the ApiKey struct at the end of models.rs:

    Read /home/ekats/Repos/Mycelica/src-tauri/src/db/models.rs
    Edit models.rs: "#[derive(Debug, Clone, Serialize, Deserialize)] pub struct P..."
Now add the `api_keys` table and CRUD methods to schema.rs. Let me find the `init()` method where tables are created:

    Grep "fn init\(&self\)" in schema.rs
    Read /home/ekats/Repos/Mycelica/src-tauri/src/db/schema.rs
    Read /home/ekats/Repos/Mycelica/src-tauri/src/db/schema.rs
    Read /home/ekats/Repos/Mycelica/src-tauri/src/db/schema.rs
Now let me find where the migrations end to add the api_keys table creation:

    Grep "Migration.*deleted_items|Migration.*merged_items|M" in schema.rs
    Read /home/ekats/Repos/Mycelica/src-tauri/src/db/schema.rs
    Read /home/ekats/Repos/Mycelica/src-tauri/src/db/schema.rs
Good. I'll add the `api_keys` table creation right before the FTS rebuild (end of init). And I'll add the CRUD methods as a new section:

    Edit schema.rs: "        // Rebuild FTS index to fix any corruption from inte..."
Now add the API key CRUD methods. Let me find a good place — after the sovereignty helpers section:

    Read /home/ekats/Repos/Mycelica/src-tauri/src/db/schema.rs
Good. I'll add API key methods right after the sovereignty helpers section. Let me find where the next section starts:

    Grep "// === " in schema.rs
    Read /home/ekats/Repos/Mycelica/src-tauri/src/db/schema.rs
Now I need to add the ApiKey model to the imports and add CRUD methods. First, update the import:

    Edit schema.rs: "use super::models::{Node, Edge, NodeType, EdgeType, Position..."
Now add the API key CRUD methods:

    Edit schema.rs: "    // === Node operations ===     pub fn insert_node(&self,..."
Now I need to export `ApiKey` from the `db` module. Let me check the module structure:

    Read /home/ekats/Repos/Mycelica/src-tauri/src/db/mod.rs
    Edit mod.rs: "pub use models::{Node, Edge, NodeType, EdgeType, Position, T..."
Good. Now the big piece: rewrite server.rs with auth middleware, `--no-auth` flag, admin commands, and author enforcement. Let me write the full updated file:

    Read /home/ekats/Repos/Mycelica/src-tauri/src/bin/server.rs
This is a significant rewrite. Let me do it with the full file to avoid edit conflicts:

    Write server.rs
Now build to check for errors:

    $ cargo +nightly build --features mcp --bin mycelica-server 2>&1 | grep "^error" || echo "BUILD SUCCES
Need to fix the extractor approach. Can't use `Request` and `Json` together. Let me use `Option<Extension<AuthContext>>` instead:

    Edit server.rs: "use axum::{     extract::{Path, Query, Request, State},     ..."
Now fix all the handler signatures to use `Option<Extension<AuthContext>>` instead of manually extracting from `Request`:

    Edit server.rs: "// POST /nodes async fn create_node_handler(     State(state..."
    Edit server.rs: "// PATCH /nodes/:id async fn patch_node_handler(     State(s..."
    Edit server.rs: "// DELETE /nodes/:id async fn delete_node_handler(     State..."
    Edit server.rs: "// POST /edges async fn create_edge_handler(     State(state..."
    Edit server.rs: "// PATCH /edges/:id async fn patch_edge_handler(     State(s..."
    Edit server.rs: "// DELETE /edges/:id async fn delete_edge_handler(     State..."
Now rebuild:

    $ cargo +nightly build --features mcp --bin mycelica-server 2>&1 | grep "^error" || echo "BUILD SUCCES
Server compiles. Let me also make sure the full project still builds:

    $ cargo +nightly build --features mcp 2>&1 | grep "^error" || echo "BUILD SUCCESS"
Step 2 compiles. Let me verify the CLI still works and test the admin commands:

    $ cargo +nightly run --features mcp --bin mycelica-server -- --help 2>&1 | grep -v "^warning"
Help output looks good. Let me fix the unused `delete` import warning and the unused `DeletedItem` warning:

    Edit server.rs: "    routing::{get, post, patch, delete},..."
    $ cargo +nightly build --features mcp --bin mycelica-server 2>&1 | grep "^error" || echo "BUILD SUCCES
Good. Now test the admin commands:

    $ cargo +nightly run --features mcp --bin mycelica-server -- admin list-keys 2>&1 | grep -v "^warning"
Works. Now test creating a key:

    $ cargo +nightly run --features mcp --bin mycelica-server -- admin create-key testuser --role editor 2
Key creation works. Let me verify it shows up in list:

    $ cargo +nightly run --features mcp --bin mycelica-server -- admin list-keys 2>&1 | tail -5
Step 2 is complete. Let me clean up the test key and mark the task done:

    $ cargo +nightly run --features mcp --bin mycelica-server -- admin revoke-key dadf8d46-58e0-4b01-a714-
    Task #2 -> completed
    Task #3 -> in_progress
Step 3: Team GUI client. Need to update `remote_client.rs` to attach auth headers, and add API key support to the team GUI. Let me read the relevant files:

    Read /home/ekats/Repos/Mycelica/src-tauri/src/remote_client.rs
Now update `RemoteClient` to support API key auth. I'll add an `api_key` field and attach the bearer header to write requests:

    Edit remote_client.rs: "pub struct RemoteClient {     base_url: String,     client: ..."
Continuing with Step 3: Team GUI client auth support. Let me check the current state of the files I need to modify.

    Read /home/ekats/Repos/Mycelica/src-tauri/src/remote_client.rs
    Glob src-tauri/src/commands/team.rs
    Glob src-team/**/*.{ts,tsx}
No `src-team/` React directory. Let me check what exists for team GUI and read the team commands.

    Read /home/ekats/Repos/Mycelica/src-tauri/src/commands/team.rs
    Glob src/**/team*
    Glob src/**/Team*
Let me check where the team GUI React code lives.

    Glob src/**/*team*
    Glob src/**/*Team*
    Glob src/**/*Settings*
    $ ls /home/ekats/Repos/Mycelica/src/
    Grep "team|Team" in src
No team React frontend exists yet — only the Rust backend (Tauri commands). I'll focus on the Rust side: `remote_client.rs` and `team.rs`.

Let me make all the changes. First, `remote_client.rs` — update constructor and add auth headers to write methods.

    Edit remote_client.rs: "impl RemoteClient {     pub fn new(base_url: &str) -> Self {..."
Now add auth headers to all write methods (POST, PATCH, DELETE). GET methods stay headerless.

    Edit remote_client.rs: "    pub async fn create_node(&self, req: &CreateNodeRequest)..."
    Edit remote_client.rs: "    pub async fn patch_node(&self, id: &str, req: &PatchNode..."
    Edit remote_client.rs: "    pub async fn create_edge(&self, req: &CreateEdgeRequest)..."
    Edit remote_client.rs: "    pub async fn patch_edge(&self, id: &str, req: &PatchEdge..."
Now update `team.rs` — add `api_key` to `TeamConfig` and pass it through to `RemoteClient`.

    Edit team.rs: "#[derive(Debug, Clone, Serialize, Deserialize)] pub struct T..."
    Edit team.rs: "        Self {             server_url: "http://localhost:374..."
Now update `TeamState::new()` and `make_client()` to use `with_api_key`.

    Edit team.rs: "        let client = RemoteClient::new(&config.server_url); ..."
Now update `make_client()` to propagate the API key.

    Edit team.rs: "    fn make_client(&self) -> Result<RemoteClient, String> { ..."
Now update `team_refresh` which also creates a `RemoteClient` directly.

    Edit team.rs: "    let base_url = {         let c = state.client.lock().map..."
Now update `team_save_settings` to use `with_api_key` when recreating the client.

    Edit team.rs: "    {         let mut client = state.client.lock().map_err(|..."
Now let's build to verify everything compiles.

    $ cargo +nightly build --features mcp 2>&1
Clean build. All pre-existing warnings, zero new errors. Step 3 is done (Rust side). No React team frontend exists to modify, so the GUI settings panel is deferred to when that gets built.

    Task #3 -> completed
    Task #4 -> in_progress
Now Step 4: CORS policy on team server. Let me read the current server.rs to find where to add the CORS layer.

    Read /home/ekats/Repos/Mycelica/src-tauri/src/bin/server.rs
    Grep "Router::new|\.layer\(|tower.http|CorsLayer" in server.rs
    Read /home/ekats/Repos/Mycelica/src-tauri/src/bin/server.rs
Now check if `tower-http` is already a dependency and what features are available.

    Grep "tower-http" in Cargo.toml
`tower-http` with `cors` feature is already a dependency. Add the CORS layer.

    Edit server.rs: "use std::sync::Arc; use std::path::PathBuf; use std::time::{..."
Now add the CORS layer to the router. The policy: allow GET from any origin (public reads), restrict write methods to configured origins or same-origin only.

    Edit server.rs: "        .layer(middleware::from_fn_with_state(state.clone(),..."
Now Step 5: Change default bind from `0.0.0.0:3741` to `127.0.0.1:3741`. Let me find the default bind address.

    Grep "0\.0\.0\.0:3741|bind_addr|default.*3741" in server.rs
    Read /home/ekats/Repos/Mycelica/src-tauri/src/bin/server.rs
    Edit server.rs: "                println!("  --bind ADDR:PORT  Bind address (..."
    Edit server.rs: "        .unwrap_or_else(|| "0.0.0.0:3741".to_string());..."
Also update the usage doc comment at the top.

    Edit server.rs: "//!   mycelica-server --db /path/to/team.db --bind 0.0.0.0:3..."
Now build to verify both Steps 4 and 5.

    $ cargo +nightly build --features mcp 2>&1 | grep -E "error|warning.*server"
Clean build. Let me check what that 1 warning is.

    $ cargo +nightly build --features mcp 2>&1 | grep -A2 "server.rs"
Pre-existing warning (`DeletedItem` unused struct from before this PR). No new warnings or errors.

    Task #4 -> completed
    Task #5 -> completed
All 5 steps complete. Let me install the CLI and server binaries.

    $ cargo +nightly install --path . --bin mycelica-cli --features mcp --force 2>&1 | tail -5
    $ cargo +nightly install --path . --bin mycelica-server --features mcp --force 2>&1 | tail -5
    $ cp ~/.cargo/bin/mycelica-cli /home/ekats/Repos/Mycelica/binaries/mycelica-cli-x86_64-unknown-linux-g
Quick verification that everything works.

    $ mycelica-cli db stats 2>&1 | head -10
    $ mycelica-server --help 2>&1 || true
Everything working. Let me do a quick end-to-end test of the admin key commands.

    $ mycelica-server --db /home/ekats/.local/share/com.mycelica.app/team/local.db admin list-keys 2>&1
    $ mycelica-server --db /home/ekats/.local/share/com.mycelica.app/team/local.db admin create-key ekats 
    $ mycelica-server --db /home/ekats/.local/share/com.mycelica.app/team/local.db admin list-keys 2>&1
    $ mycelica-server --db /home/ekats/.local/share/com.mycelica.app/team/local.db admin revoke-key e6ccbe
All admin commands work. Here's a summary of everything implemented across this session:

## Security Hardening Complete (Steps 1-5)

### Step 1: Mutex Safety (schema.rs)
- Added `Database::conn()` helper that maps `PoisonError` to `rusqlite::Error`
- Replaced 231 `.lock().unwrap()` calls with `.conn()?`
- 2 non-Result functions use `unwrap_or_else(|e| e.into_inner())` instead

### Step 2: Auth Middleware + API Keys (server.rs)
- `api_keys` table + CRUD methods in schema.rs
- `ApiKey` model in models.rs
- SHA-256 key hashing (stores only hash, never raw key)
- Axum middleware: GET passes through, POST/PATCH/DELETE require `Authorization: Bearer <key>`
- `--no-auth` flag for trusted networks (Tailscale)
- Author enforcement: server sets `author` from API key's `user_name`
- Editor ownership checks: can only modify/delete own nodes
- Admin CLI: `create-key`, `list-keys`, `revoke-key`

### Step 3: Team Client Auth (remote_client.rs + team.rs)
- `RemoteClient::with_api_key()` constructor
- `auth()` helper attaches `Authorization: Bearer <key>` to POST/PATCH/DELETE requests
- GET requests remain headerless
- `TeamConfig` gains `api_key` field (persisted in config.json)
- All team state creation/recreation passes API key through

### Step 4: CORS Policy (server.rs)
- Added `tower-http::CorsLayer` with explicit method allowlist
- `allow_origin(Any)` — auth middleware handles write protection; CORS allows Tauri webview + local dev

### Step 5: Default Bind Address (server.rs)
- Changed default from `0.0.0.0:3741` to `127.0.0.1:3741`
- Explicit `--bind 0.0.0.0:3741` required to listen on all interfaces

### Files Modified
| File | Changes |
|------|---------|
| `src-tauri/src/db/schema.rs` | `conn()` helper, 231 unwrap replacements, `api_keys` table + CRUD |
| `src-tauri/src/db/models.rs` | `ApiKey` struct |
| `src-tauri/src/db/mod.rs` | `ApiKey` export |
| `src-tauri/src/bin/server.rs` | Auth middleware, CORS, default bind, admin commands |
| `src-tauri/src/remote_client.rs` | `with_api_key()`, `auth()`, bearer headers on writes |
| `src-tauri/src/commands/team.rs` | `api_key` field in TeamConfig, pass through to RemoteClient |

### Remaining (not in this PR scope)
- Rate limiting (Step 6) — needs `tower_governor` crate
- Input validation (Step 7) — field size limits in handlers
- TLS (Step 8) — deployment/infrastructure concern (Caddy)
- Team React GUI settings panel — no React frontend exists yet

The transcript for this session is at:

```
/home/ekats/.claude/projects/-home-ekats-Repos-Mycelica/94e16c0d-55e3-4b82-a2b2-ad61b9327c2f.jsonl
```

That's the same file referenced in the continuation summary. It contains both the previous conversation (before context compaction) and this continuation.

    $ wc -l /home/ekats/.claude/projects/-home-ekats-Repos-Mycelica/94e16c0d-55e3-4b82-a2b2-ad61b9327c2f.j
    $ python3 -c "
import json, sys, textwrap

lines = open('/home/ekats/.claude/projects/-home-ekats-Repo
That's huge. Let me write it to a cleaner file for you.

