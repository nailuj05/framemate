#!/usr/bin/env bash
# Packages the prebuilt static agent as a Flatpak bundle. No flatpak-builder needed:
# nothing is compiled or run for aarch64 here, the build directory is only assembled
# (binary + metadata) and exported, so this works on an x86 desktop.
#
#   scripts/flatpak.sh            build target/flatpak/framemate-agent.flatpak
#   scripts/flatpak.sh install    ...then install it on the Frame like scripts/install.sh does
#                                 (service + self check + pairing code), from this build
#
# Env: FRAME_HOST (default steamos@frame.local), FRAME_SSH_OPTS (see deploy.sh).
set -euo pipefail

APP_ID=dev.framemate.Agent
ARCH=aarch64
RUNTIME="org.freedesktop.Platform/$ARCH/26.08"
TARGET=aarch64-unknown-linux-musl
HOST="${FRAME_HOST:-steamos@frame.local}"
read -ra SSH_OPTS <<< "${FRAME_SSH_OPTS:-}"

cd "$(dirname "$0")/.."
OUT=target/flatpak
BUILD="$OUT/build"
REPO="$OUT/repo"
BUNDLE="$OUT/framemate-agent.flatpak"

cargo build --release --target "$TARGET" -p framemate-agent

rm -rf "$BUILD"
mkdir -p "$BUILD/files/bin" "$BUILD/export"
install -m 755 "target/$TARGET/release/framemate-agent" "$BUILD/files/bin/"
# Permissions: localhost:8080 (Steam DevTools) + LAN API; steamos-manager; /dev/video*
# for the headset stream; the systemd user unit written by `install-service`.
cat > "$BUILD/metadata" <<EOF
[Application]
name=$APP_ID
runtime=$RUNTIME
command=framemate-agent

[Context]
shared=network;
devices=all;
filesystems=xdg-config/systemd/user:create;

[Session Bus Policy]
com.steampowered.SteamOSManager1=talk
org.freedesktop.systemd1=talk
EOF

flatpak build-export --arch="$ARCH" "$REPO" "$BUILD" >/dev/null
flatpak build-bundle --arch="$ARCH" --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
  "$REPO" "$BUNDLE" "$APP_ID"
echo "Built $BUNDLE"

[[ "${1:-}" == install ]] || exit 0

ssh() { /usr/bin/ssh "${SSH_OPTS[@]}" "$HOST" "$@"; }
/usr/bin/scp -q "${SSH_OPTS[@]}" "$BUNDLE" "$HOST:/tmp/framemate-agent.flatpak"
# Same steps as scripts/install.sh. The dev unit from deploy.sh would hold the ports.
ssh "systemctl --user stop framemate-agent-dev 2>/dev/null; \
  flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo && \
  flatpak install --user --reinstall --noninteractive -y /tmp/framemate-agent.flatpak && \
  rm /tmp/framemate-agent.flatpak && \
  flatpak run --user $APP_ID install-service && \
  echo && flatpak run --user $APP_ID pair"
