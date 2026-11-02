#!/usr/bin/env bash
# Installs the companion for this user: a copy of companion.ts, a `run` wrapper, and
# a LaunchAgent that keeps it running. Rerun to update. Nothing here needs root.
set -euo pipefail
bun=$(command -v bun || true)
[ -n "$bun" ] || { echo "Install Bun first." >&2; exit 1; }
home=$HOME/.local/share/hex-companion
agent=$HOME/Library/LaunchAgents/anomaly.hex.companion.plist
mkdir -p "$home" "$(dirname "$agent")"
chmod 700 "$home"
cp "$(dirname "$0")/companion.ts" "$home/companion.ts"
printf '#!/bin/sh\nexec "%s" "%s/companion.ts" "$@"\n' "$bun" "$home" > "$home/run"
chmod 700 "$home/run"
cat > "$agent" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>anomaly.hex.companion</string>
  <key>ProgramArguments</key>
  <array><string>$home/run</string><string>serve</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
</dict>
</plist>
PLIST
launchctl bootout "gui/$(id -u)/anomaly.hex.companion" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$agent"
echo "Hex companion is running on http://127.0.0.1:8787"
