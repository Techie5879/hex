# Hex companion

A small private service that lets the phone and your computer share one profile, and lets the
phone reach long-text polish without holding any provider credentials. It costs nothing to
run on a machine you already own.

- **Profile sync:** one JSON document with `ETag` and `If-Match`. Merging happens on the devices.
- **Polish:** forwards to the OpenCode service on the same machine, using OpenCode's own login.
- **Binds to `127.0.0.1` only.** `tailscale serve` is the only way in, so it's reachable from
  your tailnet over HTTPS and not from the internet. Never use `tailscale funnel` with it.
- **Bearer tokens per device.** Only hashes are stored. Delete a line in `tokens` to revoke one.
- **No request bodies in its logs.** Nothing else about your dictations reaches it unless a
  dictation passes the polish threshold.

## Set up

```sh
bun android/companion/companion.ts add-device phone   # shows the token once
bun android/companion/companion.ts add-device mac
mkdir -p ~/.config/systemd/user
cp android/companion/hex-companion.service ~/.config/systemd/user/
systemctl --user daemon-reload && systemctl --user enable --now hex-companion
sudo tailscale serve --bg --https=8443 http://127.0.0.1:8787
tailscale serve status    # shows your https://<host>.<tailnet>.ts.net:8443 address
```

`tailscale serve` needs HTTPS turned on for your tailnet. If it stops and prints a link, open it once.

In Hex on the phone, open Settings › Sync and polish:

- Profile URL: `https://<host>.<tailnet>.ts.net:8443/v1/profile`
- Polish URL: `https://<host>.<tailnet>.ts.net:8443/v1/generate`
- Token: the one from `add-device phone`

The phone needs the Tailscale app connected. Offline, edits stay on the phone and sync later,
and polish fails fast, keeping the local text.

Polish needs a running OpenCode service on the same machine, signed in to your providers:
`opencode service start`. `GET /v1/health` reports whether the companion can see it.

## Use a different server

Any WebDAV server with `ETag` and `If-Match` works for sync (for example, Nextcloud). Use the
file's full URL and `user:app-password` as the token. Polish still needs this companion.

## Why this design

| Option | Result |
|---|---|
| Companion over Tailscale | Chosen. Free, encrypted, never public, and one small file to run. |
| WebDAV | Supported, for anyone who already runs it. |
| Sync folder picked with Android's file picker | Rejected: a provider can revoke or stall the grant, and there's no compare-and-swap. Acceptable only for a one-off import. |
| Google Drive, iCloud | Rejected: needs accounts and app registration, and no shared API on both platforms. |
| New public endpoint | Rejected. |

## Test

```sh
cd android/companion && bun test
```
