# Dictionary and prompt sync

Desktop Hex hosts the shared profile while it is running. There is no separate
companion, Bun runtime, LaunchAgent, or hourly Mac process. Only the dictionary,
Global cleanup prompt and cleanup policy sync. Recordings, history, account tokens,
and other settings stay on their device.

## Set up the Mac

Use a desktop build from this branch; older installed releases do not contain the
profile host. Enable **Labs > Profile sync** and pair the phone:

```sh
hex profile device add phone --token-file "$HOME/hex-phone-token.txt"
```

The command creates a new owner-only token file and refuses to replace an existing
file. Transfer its contents privately into the phone's sync settings, then delete
the transfer copy. The host stores only its hash. `hex profile device list` shows
names, and `hex profile device revoke phone` revokes access.

The profile endpoint listens only on `127.0.0.1:8787`, only while desktop Hex is
running, Profile sync is on, and a paired device exists. It exposes no dictation or
control endpoints. Quit Hex, turn off Profile sync, or revoke every device to stop
it. The HTTP worker may take a few seconds to finish a request already underway.

Enable HTTPS for your Tailscale network, then expose the profile listener privately:

```sh
tailscale serve --bg --https=8443 http://127.0.0.1:8787
tailscale serve status
```

Use the reported address plus `/v1/profile` in **Settings > Dictionary sync** on the
phone, together with the phone token. Both devices need Tailscale connected for
this route. Do not use Tailscale Funnel. Dictation and direct ChatGPT polish do not
use this connection.

## Timing and offline use

The phone syncs after edits pause for four seconds, when Hex opens, on **Sync now**,
and in an approximately hourly Android background job. Android can delay that job
for battery management. The Mac exposes its latest local edits on the next request
and applies received changes through Hex's settings path. It does not contact the
phone or wake the Mac. Changes remain local while the Mac is asleep, Hex is closed,
or the connection is unavailable, and merge when a later attempt succeeds.

Edits use ETags and conditional writes. A stale write is refused and retried after
fetching the current profile. Concurrent prompt edits preserve the other version
as a conflict; nothing uses last-writer-wins on an entire settings file.

## Replace an installed companion

Use the new desktop binary before removing the old service. Import its profile and
existing token hashes once:

```sh
hex profile migrate-companion "$HOME/.local/share/hex-companion"
```

Import refuses to overwrite an existing hosted profile. The old directory remains
intact for rollback, and existing phone tokens continue to work. After importing,
turn on Labs > Profile sync in the new desktop app, stop the old service, and remove
its startup registration:

```sh
launchctl bootout "gui/$(id -u)/anomaly.hex.companion"
rm "$HOME/Library/LaunchAgents/anomaly.hex.companion.plist"
```

Keep the old private directory until you have verified both directions of sync.
The listener then uses the same port and Tailscale URL. Check an authenticated
`GET /v1/profile` and confirm a test edit arrives in each direction before deleting
old files. No migration changes an existing ChatGPT session or transfers it.

## Optional WebDAV

Both clients still support an existing HTTPS WebDAV file with ETag and If-Match.
Use its full file URL and `user:app-password` credentials. On desktop,
`hex profile connect <url> --token-file <file>` and `hex profile sync` remain manual
client commands. That route does not require the built-in Mac listener or Tailscale.
No external storage account is created by Hex.
