#!/bin/bash
# deploy-server.sh — Generate Caddy + systemd configs for mycelica-server
#
# Usage:
#   ./deploy-server.sh                     # Self-signed TLS on :3743
#   ./deploy-server.sh mycelica.example.ee # Let's Encrypt TLS on domain
#
# This script generates config files and prints instructions.
# It does NOT auto-run dangerous commands.

set -euo pipefail

DOMAIN="${1:-}"
MYCELICA_USER="mycelica"
MYCELICA_DB="/var/lib/mycelica/team.db"
MYCELICA_BIN="/usr/local/bin/mycelica-server"
BIND_ADDR="127.0.0.1:3741"

# --- Check prerequisites ---

echo "=== Mycelica Team Server Deployment ==="
echo

if command -v caddy &>/dev/null; then
    echo "[OK] Caddy is installed: $(caddy version 2>/dev/null || echo 'unknown version')"
else
    echo "[!!] Caddy is NOT installed. Install it:"
    echo
    echo "  sudo apt install -y debian-keyring debian-archive-keyring apt-transport-https"
    echo "  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | sudo gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg"
    echo "  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | sudo tee /etc/apt/sources.list.d/caddy-stable.list"
    echo "  sudo apt update && sudo apt install caddy"
    echo
fi

# --- Generate Caddyfile ---

CADDYFILE="/etc/caddy/Caddyfile"
echo "--- Caddyfile ($CADDYFILE) ---"
echo

if [ -n "$DOMAIN" ]; then
    cat <<EOF
# Mycelica Team Server — Let's Encrypt TLS
# Domain: $DOMAIN
$DOMAIN {
    reverse_proxy $BIND_ADDR
}
EOF
else
    cat <<EOF
# Mycelica Team Server — Self-signed TLS (LAN only)
# Access via: https://<server-ip>:3743
:3743 {
    tls internal
    reverse_proxy $BIND_ADDR
}
EOF
fi

echo
echo "To install: sudo tee $CADDYFILE <<'CADDYEOF'"
echo "  <paste the block above>"
echo "CADDYEOF"
echo

# --- Generate systemd service ---

SERVICE_FILE="/etc/systemd/system/mycelica-server.service"
echo "--- systemd service ($SERVICE_FILE) ---"
echo

cat <<EOF
[Unit]
Description=Mycelica Team Server
After=network.target

[Service]
Type=simple
User=$MYCELICA_USER
ExecStart=$MYCELICA_BIN --db $MYCELICA_DB --bind $BIND_ADDR
Restart=always
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
EOF

echo
echo "To install: sudo tee $SERVICE_FILE <<'SVCEOF'"
echo "  <paste the block above>"
echo "SVCEOF"
echo

# --- Print next steps ---

echo "=== Next Steps ==="
echo
echo "1. Create system user:"
echo "   sudo useradd -r -s /bin/false $MYCELICA_USER"
echo "   sudo mkdir -p /var/lib/mycelica"
echo "   sudo chown $MYCELICA_USER:$MYCELICA_USER /var/lib/mycelica"
echo
echo "2. Copy binary:"
echo "   sudo cp mycelica-server $MYCELICA_BIN"
echo
echo "3. Initialize database + create admin key:"
echo "   sudo -u $MYCELICA_USER $MYCELICA_BIN --db $MYCELICA_DB admin create-key <your-name> --role admin"
echo
echo "4. Install config files (see above), then:"
echo "   sudo systemctl daemon-reload"
echo "   sudo systemctl enable --now mycelica-server"
echo "   sudo systemctl enable --now caddy"
echo
if [ -n "$DOMAIN" ]; then
    echo "5. Test: curl https://$DOMAIN/health"
else
    echo "5. Test: curl -k https://localhost:3743/health"
fi
echo
echo "6. Create member keys:"
echo "   ./scripts/create-member-key.sh <username> [admin|editor]"
