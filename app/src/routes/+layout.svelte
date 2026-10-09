<script lang="ts">
  import "../app.css";
  import { onMount, type Snippet } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { agent } from "$lib/agent.svelte";
  import Battery from "$lib/components/Battery.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import UpdateNotice from "$lib/components/UpdateNotice.svelte";
  import { updates } from "$lib/updates.svelte";

  let { children }: { children: Snippet } = $props();

  const tabs = [
    { href: "/", icon: "home", label: "Home" },
    { href: "/downloads", icon: "download", label: "Downloads" },
    { href: "/view", icon: "cast", label: "Mirroring" },
    { href: "/system", icon: "performance", label: "System" },
    { href: "/settings", icon: "settings", label: "Settings" },
  ];

  const battery = $derived(agent.state?.steam.topics.battery);
  // Old data stays visible while reconnecting, dimmed and with its age in the top bar.
  const stale = $derived(!!agent.state && !agent.live);
  const lastUpdate = $derived(
    new Date(agent.receivedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }),
  );

  onMount(() => {
    // Pairing lives on the Rust side now, so the connection details arrive asynchronously.
    agent.start().then(() => {
      if (!agent.configured) goto("/settings");
    });
    if (updates.autoCheck) updates.check();
    // Coming back from the background: the socket may be dead without knowing it, so always
    // fetch a fresh snapshot instead of trusting the old one.
    const onVisible = () => {
      if (document.visibilityState === "visible") agent.refresh();
    };
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("online", onVisible);
    return () => {
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener("online", onVisible);
    };
  });
</script>

<div class="app">
  <header class="topbar">
    <img src="/favicon.svg" alt="" class="logo" />
    <a href="/" class="title">FrameMate</a>
    <span class="spacer"></span>
    {#if battery}
      <span class="battery" class:stale>
        {Math.round(battery.level * 100)}%
        <Battery level={battery.level} charging={battery.ac_state === 2} size={18} />
      </span>
    {/if}
    {#if stale}
      <span class="stale-note">{agent.status === "connecting" ? "Updating…" : lastUpdate}</span>
    {/if}
    <span class="status {agent.status}" title={agent.status}></span>
    <a href="/help" class="help" class:active={true} aria-label="Help" >
      <Icon name="help" size={20} />
    </a>
  </header>

  <main class:stale>
    {@render children()}
  </main>

  {#if updates.available && !updates.dismissed && page.url.pathname !== "/settings"}
    <UpdateNotice />
  {/if}

  <nav class="tabs">
    {#each tabs as tab}
      <a href={tab.href} class:active={page.url.pathname === tab.href} aria-label={tab.label}>
        <Icon name={tab.icon} size={26} />
      </a>
    {/each}
  </nav>
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100dvh;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: calc(env(safe-area-inset-top) + 10px) var(--gutter) 10px;
    background: var(--topbar);
  }
  .logo {
    width: 30px;
    height: 30px;
  }
  .title {
    font-weight: 600;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }
  .spacer {
    flex: 1;
  }
  .battery {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 14px;
  }
  .status {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--muted);
  }
  .status.connected {
    background: var(--green);
  }
  .status.offline,
  .status.unauthorized {
    background: var(--red);
  }
  /* Flex: on a text line, line-height pushes the icon off-center. */
  .help {
    display: flex;
  }
  main {
    flex: 1;
    overflow-y: auto;
    padding-bottom: 24px;
  }
  /* Delayed, so the usual quick refresh on resume doesn't flash. */
  main.stale,
  .battery.stale {
    opacity: 0.45;
    transition: opacity 0.2s 0.8s;
  }
  .stale-note {
    color: var(--muted);
    font-size: 13px;
    animation: appear 0.2s 0.8s both;
  }
  @keyframes appear {
    from {
      opacity: 0;
    }
  }
  .tabs {
    display: flex;
    background: var(--topbar);
    padding-bottom: env(safe-area-inset-bottom);
  }
  .tabs a {
    flex: 1;
    display: flex;
    justify-content: center;
    padding: 12px 0;
    color: var(--text);
    border-top: 2px solid transparent;
  }
  .tabs a.active {
    color: var(--accent);
    border-top-color: var(--accent);
  }
</style>
