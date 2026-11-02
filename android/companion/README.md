# Hex companion

A small private service that lets the phone and your computer share one profile. It costs nothing
to run on a machine you already own. Run it on one that stays awake, such as a Mac that doesn't sleep.

- **One JSON document** with `ETag` and `If-Match`. Merging happens on the devices.
- **Binds to `127.0.0.1` only.** `tailscale serve` is the only way in, so it's reachable from
  your tailnet over HTTPS and not from the internet. Never use `tailscale funnel` with it.
- **Bearer tokens per device.** Only hashes are stored. Delete a line in `tokens` to revoke one.
- **No request bodies in its logs.** Your dictations never reach it. Polish goes directly from the
  phone to ChatGPT.

## Set up on a Mac

```sh
android/companion/install-macos.sh     # copies the file and starts it at login
~/.local/share/hex-companion/run add-device phone   # shows the token once
~/.local/share/hex-companion/run add-device mac
tailscale serve --bg --https=8443 http://127.0.0.1:8787
tailscale serve status    # shows your https://<host>.<tailnet>.ts.net:8443 address
```

`tailscale serve` needs HTTPS turned on for your tailnet. If it stops and prints a link, open it once.

In Hex on the phone, open Settings › Dictionary sync:

- Profile URL: `https://<host>.<tailnet>.ts.net:8443/v1/profile`
- Token: the one from `add-device phone`

The phone needs the Tailscale app connected. Offline, edits stay on the phone and sync later.

## Use a different server

Any WebDAV server with `ETag` and `If-Match` works (for example, Nextcloud). Use the file's full URL
and `user:app-password` as the token.

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
