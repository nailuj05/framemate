# Troubleshooting

If you ran into any issues installing or using FrameMate, check out this file for possible fixes. If the issue persists please open an issue here on GitHub. 

The more information you can give me the better, this page will also describe how to access more info for troubleshooting. 

## Common issues

Before going any further, please ensure that you are on an up to date version of SteamOS. 
Also make sure you followed the installation guide fully.

Run `flatpak run --user dev.framemate.Agent check` on the frame (ssh / console in desktop mode) for a detailed report of what might be the issue.

## Connection issues

Make sure of the following:

- Both the frame and the phone running the app need to be in the same (local) network.
- There is no firewall or router settings blocking communication between app and frame.
- Port **7381** needs to be open on the frame, this is the case by default, if you installed or configured a firewall you'll need to open that port. (Port 7380 serves the same API unencrypted but listens only on the Frame itself, so it isn't what the app uses.)
- Check whether the frame answers: `curl -k https://frame.local:7381/healthz` should print `ok`. Pass `-k` because the agent signs its own certificate. The app checks it against the fingerprint from the pairing code instead of a certificate authority, so curl and browsers will warn about it.
- mDNS used for resolving frame.local might be unreliable in some cases, use the plain ip from the frame instead
- Android 17 (SDK 37) blocks local network access by default for apps that target it. FrameMate declares `ACCESS_LOCAL_NETWORK`, if it still won't connect grant the permission under Settings > Apps > FrameMate > Permissions > Local network (it is part of the Nearby devices group, you may have to open that submenu). If you deny it, the connection will fail silently with a timeout.
 
 Note: Some guest or mesh wifi networks may isolate devices by default, make sure that isn't the issue before proceeding.

### "FrameMate only accepts connections from the local network"

The agent rejects devices it doesn't consider part of your local network. 

That can hit unusual setups: a phone on a VPN, a guest or mesh Wi-Fi with its own subnet, or a router handing out
IPv6 addresses from several prefixes. The agent's log names the rejected address
(`journalctl --user -n 50 _COMM=framemate-agent`). To turn the check off, add
`Environment=FRAMEMATE_ALLOW_REMOTE=1` under `[Service]` in
`~/.config/systemd/user/framemate-agent.service`, then run
`systemctl --user daemon-reload && systemctl --user restart framemate-agent`. 

I tried to keep this very loose, if you think your setup isnt that unsual and should not be blocked, 
please also open an issue with your details, I can improve the check.

## Installation issues

If you encounter an error during installation please send me the logs and the commands you ran in a github issue.


Should the app say the token was rejected, pair again with `flatpak run --user dev.framemate.Agent pair`.
Reinstalling keeps the token and the encryption key. Both live in the app's config directory, which Flatpak leaves
alone, so an existing pairing survives updates and uninstalls.

If you want a clean slate uninstall run

```sh
flatpak uninstall --user --delete-data dev.framemate.Agent
```


To replace the token run `flatpak run --user dev.framemate.Agent rotate-token` 
Needs a new `pair` afterwards.

If the app says the Frame isn't the one it was paired with, the agent's encryption key changed.
That happens if its config directory was wiped. Run `pair` and scan again.

### Installing from Desktop Mode

If `install-service` says "systemd isn't reachable from this terminal": Desktop Mode on the Frame is a nested desktop without access to your user's systemd, so `install-service` can't start the agent from there. The agent is still installed and starts with the next restart of the Frame.

**Scan the pairing code straight away regardless.** Pairing doesn't need the agent, so the app shows the Frame as
offline and connects on its own once it is running.

If you already restarted and the QR-Code is gone run `flatpak run --user dev.framemate.Agent pair` to print it again.

To start the agent right away instead, run the command it prints:

```sh
env XDG_RUNTIME_DIR=/run/user/$(id -u) DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$(id -u)/bus \
  flatpak run --user dev.framemate.Agent install-service
```

Installing over SSH doesn't have this problem.

## Other issues

You encountered a different issue or think you found a bug, please let me know!
Include steps to reproduce, the output of the agents `check` command and the logs (`journalctl --user -n 200 _COMM=framemate-agent`).
