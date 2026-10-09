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

# dont display escaped and control characters
transaction() {
  if { true </dev/tty; } 2>/dev/null; then
    flatpak "$@" </dev/tty
  else
    FLATPAK_FANCY_OUTPUT=0 flatpak "$@"
  fi
}

uninstall() {
  if flatpak list --user --app --columns=application | grep -qx "$APP_ID"; then
    agent uninstall-service || say "Couldn't remove the service; continuing."
    # No --delete-data: the token and TLS key stay, so reinstalling doesn't force a re-pair.
    transaction uninstall --user -y "$APP_ID"
    say "Removed FrameMate."
  else
    say "FrameMate isn't installed for this user; nothing to do."
  fi
}

main() {
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
  transaction install --user -y --reinstall "$tmp/$BUNDLE"

  say ""

  # Register the unit, restart the agent and run the self check
  # In Desktop Mode it can't reach systemd, say so, recommend reboot
  service_ok=1
  agent install-service || service_ok=0

  say ""
  agent pair

  # Ask the agent itself whether it came up; the pairing code above stays valid either way.
  if [ "$service_ok" -eq 0 ]; then
    say ""
    say "NOTE: the self check above found a problem. Once it's sorted, the app connects with this code."
    say "Repeat the check with: flatpak run --user $APP_ID check"
  elif ! curl -fsS --max-time 5 -o /dev/null "http://127.0.0.1:7380/healthz" 2>/dev/null; then
    say ""
    say "NOTE: the agent isn't running yet, which is normal in Desktop Mode installs."
    say "Scan the code above now anyway: The app will remember the fingerprint and connect to your headset once it's back up."
    say "Reboot your headset of the service to start."
  fi
}

main "$@"
