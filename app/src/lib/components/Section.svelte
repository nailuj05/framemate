<script lang="ts">
  import type { Snippet } from "svelte";
  let {
    title,
    aside,
    children,
    collapsible = false,
    open = $bindable(!collapsible),
  }: { title: string; aside?: Snippet; children: Snippet; collapsible?: boolean; open?: boolean } = $props();
</script>

<section>
  {#if collapsible}
    <button class="header" aria-expanded={open} onclick={() => (open = !open)}>
      <h2><span class="chevron" class:open></span>{title}</h2>
      {@render aside?.()}
    </button>
  {:else}
    <header class="header">
      <h2>{title}</h2>
      {@render aside?.()}
    </header>
  {/if}
  {#if open}
    {@render children()}
  {/if}
</section>

<style>
  section {
    margin: 20px 0 0;
  }
  .header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    width: 100%;
    padding: 0 var(--gutter) 8px;
  }
  button.header {
    background: none;
    border: none;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  h2 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text);
  }
  /* Small right-pointing chevron, turned down when open. */
  .chevron {
    display: inline-block;
    width: 6px;
    height: 6px;
    margin: 0 8px 1px 0;
    border-right: 2px solid currentColor;
    border-bottom: 2px solid currentColor;
    transform: rotate(-45deg);
    transition: transform 0.15s;
  }
  .chevron.open {
    transform: rotate(45deg);
  }
</style>
