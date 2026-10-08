#!/bin/sh
# FrameMate agent installer for the Steam Frame. Installs the Flatpak, registers the user service and prints the pairing QR code for the app.
#
#   curl -LsSf https://raw.githubusercontent.com/nailuj05/framemate/main/scripts/install.sh | sh
#
# Re-run it to update. To remove everything again:
#
#   curl -LsSf https://raw.githubusercontent.com/nailuj05/framemate/main/scripts/install.sh | sh -s -- uninstall
#
# Same thing as running the commands from read me manually, just quicker

set -eu

APP_ID=dev.framemate.Agent
BUNDLE=framemate-agent.flatpak
RELEASE=https://github.com/nailuj05/framemate/releases/latest/download

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

agent() { flatpak run --user "$APP_ID" "$@"; }

uninstall() {
  if flatpak info --user "$APP_ID" >/dev/null 2>&1; then
    agent uninstall-service || say "Couldn't remove the service; continuing."
    # No --delete-data: the token and TLS key stay, so reinstalling doesn't force a re-pair.
    flatpak uninstall --user -y "$APP_ID"
    say "Removed FrameMate."
  else
    say "FrameMate isn't installed for this user; nothing to do."
  fi
}

# The agent lives in the user's Flatpak installation and a systemd *user* unit, so as root
# all of it would land in the wrong place.
[ "$(id -u)" -ne 0 ] || die "run this as your normal user, not as root."
command -v flatpak >/dev/null || die "flatpak is not installed."

case "${1:-install}" in
  install) ;;
  uninstall) uninstall; exit 0 ;;
  *) die "unknown command '$1' (install, uninstall)" ;;
esac

arch=$(uname -m)
[ "$arch" = aarch64 ] || die "the agent runs on the Steam Frame (aarch64), not on $arch.
Run this in a terminal on the Frame via SSH, or use the Desktop Mode Konsole."

command -v curl >/dev/null || die "curl is not installed."

# A fresh Frame may not have the remote the Freedesktop runtime comes from.
flatpak remote-add --user --if-not-exists \
  flathub https://dl.flathub.org/repo/flathub.flatpakrepo >/dev/null

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

say "Downloading the agent..."
curl -fL --progress-bar -o "$tmp/$BUNDLE" "$RELEASE/$BUNDLE" ||
  die "download failed. Check the network, or grab $BUNDLE from the releases page by hand."

say ""
say "Installing. Pulling Freedesktop Runtime from Flathub the first time (about 270 MB), may take a while."

# --reinstall so re-running this script updates an existing install.
flatpak install --user -y --reinstall "$tmp/$BUNDLE"

say ""

# Register the unit
# Restart agent
# Runs the self check
# In Desktop Mode it cant reach systemd, says so, and still exits 0 with the unit enabled for the next boot.
agent install-service

say ""
agent pair

# install-service exits 0 whether or not the agent actually came up, so ask the agent itself.
# The pairing code above stays valid either way; only connecting has to wait.
if ! curl -fsS --max-time 5 -o /dev/null "http://127.0.0.1:7380/healthz" 2>/dev/null; then
  say ""
  say "NOTE: the agent isn't running yet, which is normal in Desktop Mode installs."
  say "Scan the code above now anyway: The app will remember the fingerprint and connect to your headset once it's back up."
fi
