<script lang="ts">
  import { agent, localNetworkBlocked } from "$lib/agent.svelte";
  import Row from "$lib/components/Row.svelte";
  import Section from "$lib/components/Section.svelte";
  import { openExternal } from "$lib/external";
  import { APP_VERSION, updates } from "$lib/updates.svelte";

  let code = $state("");
  let failure = $state<string | null>(null);
  let cameraBlocked = $state(false);

  function describe(error: unknown) {
    if (typeof error === "string") return error;
    if (error instanceof Error) return error.message;
    const message = (error as { message?: unknown } | null)?.message;
    return typeof message === "string" ? message : JSON.stringify(error);
  }

  const statusText = $derived(
    {
      unconfigured: "Not paired yet",
      connecting: "Connecting…",
      connected: `Connected to ${agent.state?.agent.hostname ?? "the Frame"}`,
      offline: "Can't reach the Frame. Is it awake and on the same network?",
      unauthorized: "The Frame rejected the token. Pair again.",
    }[agent.status],
  );

  async function pair(payload: string) {
    failure = null;
    try {
      await agent.pair(payload);
      code = "";
    } catch (error) {
      failure = describe(error);
    }
  }

  async function scanCode() {
    failure = null;
    cameraBlocked = false;
    try {
      const scanner = await import("@tauri-apps/plugin-barcode-scanner");
      // The plugin's scan() refuses without the permission and never asks for it itself.
      let permission = await scanner.checkPermissions();
      if (permission !== "granted") permission = await scanner.requestPermissions();
      if (permission !== "granted") {
        // After a "don't ask again" Android answers "denied" without showing a dialog.
        cameraBlocked = true;
        return;
      }
      const result = await scanner.scan({ windowed: false, formats: [scanner.Format.QRCode] });
      await pair(result.content);
    } catch (error) {
      const message = describe(error);
      if (message !== "cancelled") failure = message;
    }
  }
</script>

<Section title="Connection">
  <form onsubmit={event => { event.preventDefault(); pair(code); }}>
    <button class="button" type="button" onclick={scanCode}>Scan pairing code</button>
    <p class="hint">
      Run <code>flatpak run --user dev.framemate.Agent pair</code> on the Frame and scan the QR code.
    </p>
    <label>
      <span>Or enter the code it prints</span>
      <input bind:value={code} placeholder="FM1 frame.local 7381 …" autocapitalize="characters" autocorrect="off" spellcheck="false" />
    </label>
    <button class="button secondary" type="submit" disabled={!code.trim()}>Pair</button>
    {#if failure}
      <p class="status offline">{failure}</p>
    {/if}
    {#if cameraBlocked}
      <p class="status offline">
        Scanning needs the camera. Allow <b>Camera</b> in the app's permissions, or enter the code by hand.
      </p>
      <button class="button secondary" type="button" onclick={() => window.FrameMateAndroid?.openAppSettings?.()}>
        Open app settings
      </button>
    {/if}
    {#if agent.error}
      <p class="status offline">{agent.error}</p>
    {/if}
    {#if agent.status === "offline" && localNetworkBlocked()}
      <p class="status offline">
        Android blocks FrameMate from your local network. Allow <b>Nearby devices</b> in the app's permissions.
      </p>
      <button class="button secondary" type="button" onclick={() => window.FrameMateAndroid?.openAppSettings?.()}>
        Open app settings
      </button>
    {:else}
      <p class="status {agent.status}">{statusText}</p>
    {/if}
  </form>
</Section>

<Section title="Updates">
  <Row label="App version" value={APP_VERSION} />
  <Row label="Agent version" value={agent.state?.agent.version ?? "not connected"} />
  <Row label="Latest release">
    {#if updates.checking}
      Checking…
    {:else if updates.error}
      <span class="error">{updates.error}</span>
    {:else if updates.latest}
      v{updates.latest.version}
      {#if updates.available}<span class="badge">new</span>{:else}· up to date{/if}
    {:else}
      not checked
    {/if}
  </Row>
  <label class="row toggle">
    <span>Check for updates on start</span>
    <input
      type="checkbox"
      role="switch"
      checked={updates.autoCheck}
      onchange={event => updates.setAutoCheck(event.currentTarget.checked)}
    />
  </label>
  <div class="actions">
    {#if updates.available && updates.latest}
      <p class="hint">
        {#if updates.appOutdated}Download the new <code>framemate.apk</code> from the release page.{/if}
        {#if updates.agentOutdated}Update the agent on the Frame: download the new
          <code>framemate-agent.flatpak</code> and run the install commands again (see Help).{/if}
      </p>
      <button class="button" onclick={() => openExternal(updates.latest!.url)}>Open release page</button>
    {/if}
    <button class="button secondary" onclick={() => updates.check()} disabled={updates.checking}>
      {updates.checking ? "Checking…" : "Check for updates"}
    </button>
  </div>
</Section>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 16px var(--gutter);
    background: var(--surface);
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  label span {
    font-size: 13px;
    color: var(--muted);
  }
  input {
    min-height: 44px;
    padding: 0 12px;
    background: var(--surface-2);
    border: 1px solid transparent;
    border-radius: 2px;
  }
  input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .hint {
    margin: 0;
    font-size: 13px;
    color: var(--muted);
  }
  .status {
    margin: 0;
    color: var(--muted);
  }
  .status.connected {
    color: var(--green);
  }
  .status.offline,
  .status.unauthorized {
    color: var(--red);
  }
  .error {
    color: var(--red);
  }
  .badge {
    margin-left: 6px;
    padding: 1px 6px;
    border-radius: 2px;
    background: var(--accent);
    color: #fff;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
  }
  /* Same look as Row, with a SteamOS-style switch on the right. */
  .toggle {
    display: flex;
    flex-direction: row;
    justify-content: space-between;
    align-items: center;
    gap: 16px;
    min-height: 48px;
    padding: 0 var(--gutter);
    background: var(--surface);
    border-bottom: 1px solid var(--divider);
  }
  .toggle span {
    font-size: inherit;
    color: var(--text);
  }
  .toggle input {
    appearance: none;
    position: relative;
    flex: none;
    width: 44px;
    min-height: 0;
    height: 24px;
    padding: 0;
    border: 0;
    border-radius: 12px;
    background: var(--track);
    cursor: pointer;
    transition: background 0.15s;
  }
  .toggle input::after {
    content: "";
    position: absolute;
    top: 3px;
    left: 3px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: #fff;
    transition: transform 0.15s;
  }
  .toggle input:checked {
    background: var(--accent);
  }
  .toggle input:checked::after {
    transform: translateX(20px);
  }
  .actions {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 16px var(--gutter);
    background: var(--surface);
  }
  .button.secondary {
    background: var(--surface-2);
  }
  .button:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>
