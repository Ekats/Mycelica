#!/bin/bash
# create-member-key.sh — Create an API key for a team member
#
# Usage: ./create-member-key.sh <username> [admin|editor]
# Default role: editor

set -euo pipefail

if [ $# -lt 1 ]; then
    echo "Usage: $0 <username> [admin|editor]"
    echo "  Default role: editor"
    exit 1
fi

USERNAME="$1"
ROLE="${2:-editor}"
DB="/var/lib/mycelica/team.db"

if [ "$ROLE" != "admin" ] && [ "$ROLE" != "editor" ]; then
    echo "Error: role must be 'admin' or 'editor', got '$ROLE'"
    exit 1
fi

echo "Creating $ROLE key for: $USERNAME"
mycelica-server --db "$DB" admin create-key "$USERNAME" --role "$ROLE"
