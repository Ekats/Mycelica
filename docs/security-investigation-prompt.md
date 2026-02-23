# Security Investigation: Mycelica Team GUI + Server

## Context

Mycelica is being set up for a hackerspace. The original team mode plan assumed 4 trusted people on a private Tailscale network where "if you're on the network, you're authorized." A hackerspace changes the threat model — semi-trusted users, shared network, unknown devices. We need application-level security before deployment.

**Do NOT implement anything yet.** This is investigation only. Use `plan` mode. Spawn explore agents for parallel investigation.

## Investigation Tasks

### Agent 1: Current Auth & Security State

Investigate what security measures exist (if any) and what's exposed:

```bash
# Check if there's any existing auth code
grep -rn "auth\|token\|password\|api_key\|bearer\|session\|login" src-tauri/src/ --include="*.rs" | grep -v target/ | grep -v "// "

# Check the HTTP server for auth
cat src-tauri/src/http_server.rs

# Check if there's a users table or any auth schema
grep -rn "users\|CREATE TABLE.*user\|api_key\|role\|permission" src-tauri/src/db/ --include="*.rs"

# Check what endpoints exist and whether any have access control
grep -rn "fn handler\|async fn handle\|Router\|.route(" src-tauri/src/ --include="*.rs" | grep -v target/

# Check Tauri commands for any auth checks
grep -rn "#\[tauri::command\]" src-tauri/src/commands/ --include="*.rs" -A 3

# Check if the team server binary exists yet or is still planned
ls src-tauri/src/bin/
cat src-tauri/Cargo.toml | grep -A 5 "\[\[bin\]\]"

# Check for hardcoded secrets, keys, or credentials
grep -rn "secret\|hardcoded\|TODO.*auth\|FIXME.*security\|HACK" src-tauri/src/ --include="*.rs"
```

**Report:** What auth exists today? What's exposed without protection? Is the axum server binary implemented or still planned?

### Agent 2: Database Schema & Input Handling

Investigate the current schema, how inputs are handled, and where injection/corruption risks exist:

```bash
# Full schema — every CREATE TABLE, every column
cat src-tauri/src/db/schema.rs

# Check for parameterized queries vs string formatting
grep -rn "format!\|&format\|execute(" src-tauri/src/db/ --include="*.rs" | head -40

# Check if any queries use string interpolation (SQL injection risk)
grep -rn "format!.*SELECT\|format!.*INSERT\|format!.*UPDATE\|format!.*DELETE" src-tauri/src/db/ --include="*.rs"

# Check input validation — are there any size limits, sanitization?
grep -rn "max_len\|truncate\|sanitize\|validate\|limit\|MAX_" src-tauri/src/ --include="*.rs"

# Check the author field — is it on nodes? edges? both?
sqlite3 ~/.local/share/com.mycelica.app/team/local.db ".schema" 2>/dev/null || echo "team db not found"
sqlite3 .mycelica.db ".schema" 2>/dev/null || echo "personal db not found"

# Check models for author, human_edited, sovereignty fields
cat src-tauri/src/db/models.rs

# Check if content is stored as raw text or sanitized
grep -rn "content\|body\|description" src-tauri/src/db/schema.rs
```

**Report:** Is SQL parameterized everywhere? Are there size limits on any fields? Does the author field exist on nodes and edges? What's the full current schema? Any content sanitization?

### Agent 3: Unwrap Panics & Server Stability

The codebase has 50+ `unwrap()` calls. In a multi-user server, these become denial-of-service vectors (one bad request panics the server for everyone). Map the critical ones:

```bash
# Count total unwraps
grep -rn "\.unwrap()" src-tauri/src/ --include="*.rs" | grep -v target/ | grep -v test | wc -l

# Find unwraps on database locks (most critical — poison the lock, server dies)
grep -rn "\.lock()\.unwrap()\|\.read()\.unwrap()\|\.write()\.unwrap()" src-tauri/src/ --include="*.rs" | grep -v target/

# Find unwraps in HTTP handlers / Tauri commands (crash on bad input)
grep -rn "unwrap()" src-tauri/src/http_server.rs src-tauri/src/commands/ --include="*.rs" | grep -v target/

# Find unwraps on parse operations (user-controlled input)
grep -rn "parse.*unwrap\|from_str.*unwrap\|to_string.*unwrap" src-tauri/src/ --include="*.rs" | grep -v target/

# Check if there's any catch_unwind or panic handling
grep -rn "catch_unwind\|panic\|set_hook" src-tauri/src/ --include="*.rs" | grep -v target/

# Check the connection type — is it Arc<Mutex<Connection>>, Arc<RwLock<Connection>>, or something else?
grep -rn "Arc.*Mutex\|Arc.*RwLock\|Connection" src-tauri/src/lib.rs src-tauri/src/db/ --include="*.rs" | head -20
```

**Report:** How many unwraps total? How many on lock acquisition? How many in request handler paths? What's the DB connection sharing pattern? Is there any panic recovery?

### Agent 4: Network Exposure & CORS

Check what's currently network-accessible and how:

```bash
# What does the HTTP server bind to?
grep -rn "bind\|listen\|0\.0\.0\.0\|127\.0\.0\.1\|localhost\|MYCELICA_BIND" src-tauri/src/ --include="*.rs"

# CORS configuration
grep -rn "cors\|CORS\|Access-Control\|Origin" src-tauri/src/ --include="*.rs"

# Check if Tauri's dev server exposes anything
cat src-tauri/tauri.conf.json | grep -A 10 "devUrl\|dangerousRemoteUrl\|security"

# Check team config if it exists
cat src-tauri/tauri.team.conf.json 2>/dev/null || echo "no team config"

# Check for any websocket endpoints
grep -rn "websocket\|ws://\|wss://\|upgrade" src-tauri/src/ --include="*.rs"

# Check what ports are referenced anywhere
grep -rn "port\|:3741\|:9876\|:8080\|:443" src-tauri/src/ --include="*.rs"
```

**Report:** What's currently listening on the network? What IPs does it bind to? Is CORS configured? What ports are in use?

### Agent 5: Dependency Audit & Supply Chain

Check dependencies for known vulnerabilities and unnecessary attack surface:

```bash
# Full dependency list
cat src-tauri/Cargo.toml

# Check for security-relevant crates already in use
grep -E "argon2|bcrypt|sha2|hmac|ring|rustls|native-tls|reqwest|hyper|axum|tower" src-tauri/Cargo.toml

# Run cargo audit if available
cargo audit 2>/dev/null || echo "cargo-audit not installed"

# Check Cargo.lock for dependency count
wc -l src-tauri/Cargo.lock 2>/dev/null || wc -l Cargo.lock 2>/dev/null

# Check if tiny_http or axum is used for the server
grep -rn "tiny_http\|axum\|actix\|warp\|rocket" src-tauri/Cargo.toml src-tauri/src/ --include="*.rs"

# Check for any TLS/SSL configuration
grep -rn "tls\|ssl\|certificate\|rustls\|native_tls" src-tauri/src/ --include="*.rs" src-tauri/Cargo.toml
```

**Report:** What HTTP framework is the server using? Are any crypto/auth crates already present? Any known vulnerabilities? Is TLS configured anywhere?

### Agent 6: Team Mode Implementation Status

Figure out exactly what's been built vs what's still planned from the team mode phases:

```bash
# Check if the team GUI exists
ls src-team/ 2>/dev/null && find src-team/ -name "*.tsx" -o -name "*.ts" | head -20 || echo "no src-team/"

# Check for team-specific Rust code
grep -rn "team\|Team" src-tauri/src/ --include="*.rs" -l | grep -v target/

# Check for the --remote flag on CLI
grep -rn "remote\|Remote" src-tauri/src/bin/cli.rs

# Check if remote_client.rs exists
ls src-tauri/src/remote_client.rs 2>/dev/null || echo "no remote_client.rs"

# Check if server binary exists
ls src-tauri/src/bin/server.rs 2>/dev/null || echo "no server binary"

# Check feature flags
grep -rn "features\|cfg.*feature\|#\[cfg" src-tauri/Cargo.toml src-tauri/src/lib.rs --include="*.rs" --include="*.toml" | head -20

# Check settings structure
cat src-tauri/src/settings.rs | head -60

# Check if signal import exists (was mentioned in recent chats)
grep -rn "signal\|Signal" src-tauri/src/ --include="*.rs" -l | grep -v target/

# What team-related commands exist in CLI?
grep -rn "add\|concept\|ask\|decide\|link\|orphan" src-tauri/src/bin/cli.rs | head -30
```

**Report:** What team features are actually implemented in code vs just planned in docs? Does the server binary exist? Does the team GUI exist? What's the gap between the plan docs and reality?

## Synthesis

After all agents report back, produce a single document:

### 1. Current Security Posture
- What's protected today (if anything)
- What's exposed
- Critical vulnerabilities ranked by severity

### 2. Gap Analysis
- What needs to exist for hackerspace deployment
- What already exists that can be reused
- What's partially built and needs finishing

### 3. Implementation Recommendations
For each security layer, note:
- What crate(s) to use
- Where in the codebase it integrates
- Estimated complexity (trivial / moderate / significant)
- Dependencies on other work

### 4. Suggested Implementation Order
Sequence the work so that:
- The most critical security gaps are closed first
- Each step is independently testable
- The server is never deployed in an insecure intermediate state

### 5. Open Questions
Flag anything ambiguous that needs human decision before implementing.
