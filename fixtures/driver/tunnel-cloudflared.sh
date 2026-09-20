#!/bin/sh
# Reference tunnel adapter: expose a running driver through a cloudflared quick tunnel.
# The driver never knows about the tunnel; any forwarder works (ngrok http <port>, tailscale funnel <port>, bore, ssh -R ...).
# Usage: driver/tunnel-cloudflared.sh [target-url]   default http://localhost:8770 (simulator); use http://<phone-ip>:8770 for a device
TARGET="${1:-http://localhost:8770}"; LOG=/tmp/agent-mobile-cloudflared.log
nohup cloudflared tunnel --url "$TARGET" > "$LOG" 2>&1 &
echo "$!" > /tmp/agent-mobile-cloudflared.pid
for i in $(seq 1 30); do grep -q "trycloudflare.com" "$LOG" && break; sleep 2; done
URL=$(grep -o "https://[a-z0-9-]*\.trycloudflare\.com" "$LOG" | head -1)
[ -n "$URL" ] && echo "$URL" > /tmp/agent-mobile-tunnel-url.txt
echo "cloudflared pid $(cat /tmp/agent-mobile-cloudflared.pid) -> $TARGET  public: ${URL:-<no url yet, see $LOG>}"
