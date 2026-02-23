CARGO    = cargo +nightly
ARCH     = $(shell rustc -vV | grep host | cut -d' ' -f2)
SIDECAR  = src-tauri/binaries/mycelica-cli-$(ARCH)

# Override with: make cli CLI_FEATURES=mcp,signal,cuda
CLI_FEATURES    ?= mcp
SERVER_FEATURES ?= team
TEAM_FEATURES   ?= team

.PHONY: all cli server spore sidecar app team \
        dev dev-team check test clean

# Default: headless binaries (fast, no webkit deps)
all: cli server spore

# --- Headless binaries (cargo install to ~/.cargo/bin) ---

cli:
	@mkdir -p $(dir $(SIDECAR))
	@test -f $(SIDECAR) || touch $(SIDECAR)
	cd src-tauri && $(CARGO) install --path . --bin mycelica-cli \
		--features $(CLI_FEATURES) --force
	cp ~/.cargo/bin/mycelica-cli $(SIDECAR)

server:
	cd src-tauri && $(CARGO) install --path . --bin mycelica-server \
		--features $(SERVER_FEATURES) --force

spore:
	cd spore && go build -o spore .

sidecar: cli

# --- Tauri apps (slow — builds frontend + webview bundle) ---
# Both run from repo root. Tauri handles npm build via beforeBuildCommand.

app: sidecar
	$(CARGO) tauri build

team: sidecar
	$(CARGO) tauri build --config tauri.team.conf.json --features $(TEAM_FEATURES)

# --- Dev mode ---

dev: sidecar
	$(CARGO) tauri dev

dev-team: sidecar
	$(CARGO) tauri dev --config tauri.team.conf.json --features team

# --- Quality ---

check:
	cd src-tauri && $(CARGO) check --all-targets --features $(CLI_FEATURES)

test:
	cd src-tauri && $(CARGO) test --features $(CLI_FEATURES)
	cd spore && go test ./...

clean:
	cd src-tauri && cargo clean
	rm -f spore/spore $(SIDECAR)
