<script lang="ts">
  import Section from "$lib/components/Section.svelte";
  import Row from "$lib/components/Row.svelte";
  import { external } from "$lib/external";
</script>

<Section title="Installation">
  <p>
    This app needs a helper called framemate-agent, a small service running on the Frame. <br>
    Open a terminal on the Frame — over SSH, or through the Desktop Mode Terminal — and run the
    installer from the GitHub page. It installs the agent, starts it, and prints a QR code. <br>
    <b>Check out the GitHub page for exact install instructions</b>
  </p>
</Section>

<Section title="Pairing">
  <p>
    Tap <b>Scan pairing code</b> in Settings and point the camera at the QR code the installer
    printed. Print it again any time with <code>flatpak run --user dev.framemate.Agent pair</code>.
    Pairing doesn't need the agent to be running, so it's always safe to scan right away: the app
    shows the Frame as offline until the agent is up, then connects on its own.
    The code carries the Frame's address, the access token and the fingerprint of the Frame's
    encryption key, so one scan is all the setup there is.
  </p>
  <p>
    The code is a secret: anyone who has it can read your headset's screen. Don't share a photo
    of it. If the terminal is too narrow for the QR code, the same line of text is printed below
    it and can be typed into Settings instead.
  </p>
</Section>

<Section title="Connecting">
  <p>
    Both headset and phone need to be on the same local network. The app reaches the Frame by its
    .local name and falls back to the IP address from the pairing code, so a router that doesn't
    forward mDNS is handled automatically.
  </p>
  <p>
    Traffic between phone and Frame is encrypted and tied to the key fingerprint from the pairing
    code. Updating or reinstalling the agent keeps that key, so pairing survives it. If the key
    does change, because the agent's configuration was wiped, the app refuses to connect and
    asks you to pair again rather than trusting a new key silently.
  </p>
</Section>

<Section title="Notifications in VR">
  <p>
    FrameMate can show your phone's notifications in the headset, as regular Steam toasts. It's off
    by default: tap the bell in the top bar to turn it on, the crossed out bell means it's off.
    The first time, Android asks you to give FrameMate notification access. After that every app's
    notifications get forwarded, you can switch off the ones you don't want in Settings (apps show
    up there once they've posted a notification).
  </p>
  <p>
    Your notifications only go to your Frame, encrypted, and are never stored. If someone else is
    using the headset, just tap the bell again and nothing gets forwarded.
  </p>
  <p>
    Android greys out the notification access switch for sideloaded apps. Open the app info, tap ⋮
    and choose <b>Allow restricted settings</b>, then try again. More in the
    <a href="https://github.com/nailuj05/framemate/blob/main/TROUBLESHOOTING.md#notifications" onclick={external}>troubleshooting guide</a>.
  </p>
</Section>

<Section title="Issues">
  <p>
    If you encounter any issues with the app or agent, please submit a GitHub Issue so I can get it fixed. <br>
    <a href="https://github.com/nailuj05/framemate/issues/new" onclick={external}>Submit an Issue here</a>
  </p>
</Section>

<style>
 p {
   min-height: 48px;
   padding: 8px var(--gutter);
   background: var(--surface);
   border-bottom: 1px solid var(--divider);
   color: var(--text);
 }
 a {
   color: var(--accent);
 }
</style>
