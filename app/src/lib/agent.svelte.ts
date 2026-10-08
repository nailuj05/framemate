// Connection to the agent: latest state from `/api/ws`, reconnects on its own.
//
// The socket goes to the app's own loopback proxy (src-tauri/src/proxy.rs), which holds the
// pinned TLS connection to the Frame, a WebView can't pin a certificate itself. Hence
// `127.0.0.1` here and the `s=` secret on every request.

import { invoke } from "@tauri-apps/api/core";

import type { AgentState } from "./types";

const RETRY_MS = 3000;
/** A connect that hasn't opened by then is given up (a sleeping Frame never answers the SYN). */
const CONNECT_TIMEOUT_MS = 8000;
/** The agent pushes at least every ~10 s (power poll); silence beyond this means a dead socket. */
const SILENCE_MS = 25000;

/** Where the proxy listens, and the two secrets every request carries. */
export interface Connection {
  port: number;
  /** Gates the proxy, so other apps on the phone can't use it to reach the LAN. */
  secret: string;
  /** The agent's own token, from the pairing code. */
  token: string;
  paired: boolean;
}

/** `unauthorized`: the agent answered but rejected the token. */
export type ConnectionStatus = "unconfigured" | "connecting" | "connected" | "offline" | "unauthorized";

class Agent {
  connection = $state<Connection | null>(null);
  state = $state<AgentState | null>(null);
  status = $state<ConnectionStatus>("unconfigured");

  /** When the last message arrived; tells live data from a snapshot left over from before. */
  receivedAt = $state(0);

  /** Something the user has to act on — above all, a Frame whose key no longer matches. */
  error = $state<string | null>(null);

  #socket: WebSocket | null = null;
  #retry: ReturnType<typeof setTimeout> | undefined;
  #watchdog: ReturnType<typeof setTimeout> | undefined;

  /** True while `state` is being kept current by an open connection. */
  get live() {
    return this.status === "connected";
  }

  get configured() {
    return this.connection?.paired ?? false;
  }

  get authority() {
    return this.connection ? `127.0.0.1:${this.connection.port}` : "";
  }

  socketUrl(path: string) {
    return `ws://${this.authority}${path}?${this.#query()}`;
  }

  #query() {
    const { token, secret } = this.connection!;
    return `token=${encodeURIComponent(token)}&s=${encodeURIComponent(secret)}`;
  }

  /** Picks up an existing pairing and connects. Resolves before the socket opens. */
  async start() {
    try {
      this.connection = await invoke<Connection | null>("connection");
    } catch {
      // Not running under Tauri (plain `deno task dev` in a browser).
      this.connection = null;
    }
    this.connect();
  }

  /** Takes a scanned or pasted pairing code; throws with a message worth showing. */
  async pair(payload: string) {
    this.connection = await invoke<Connection>("pair", { payload });
    this.error = null;
    this.state = null;
    this.status = "connecting";
    this.connect();
  }

  connect() {
    this.#close();
    if (!this.configured) {
      this.status = "unconfigured";
      return;
    }
    // Retries keep showing the last failure instead of flickering back to "connecting".
    if (this.status !== "offline" && this.status !== "unauthorized") this.status = "connecting";
    const socket = new WebSocket(this.socketUrl("/api/ws"));
    this.#socket = socket;
    let opened = false;
    const lost = () => this.#lost(socket, opened);
    socket.onmessage = event => {
      if (this.#socket !== socket) return;
      this.state = JSON.parse(event.data);
      this.status = "connected";
      this.error = null;
      this.receivedAt = Date.now();
      this.#arm(socket, SILENCE_MS, lost);
    };
    socket.onopen = () => (opened = true);
    socket.onclose = lost;
    this.#arm(socket, CONNECT_TIMEOUT_MS, lost);
  }

  /**
   * Fresh snapshot after the app was in the background: Android freezes the WebView, so the old
   * socket may be dead without ever having fired `onclose` and would keep showing old data.
   * The current state stays visible (marked stale via `live`) until the new one arrives.
   */
  refresh() {
    if (!this.configured) return;
    if (this.status === "connected") this.status = "connecting";
    this.connect();
  }

  /** Give up on `socket` if nothing arrives within `ms`. */
  #arm(socket: WebSocket, ms: number, lost: () => void) {
    clearTimeout(this.#watchdog);
    this.#watchdog = setTimeout(() => {
      if (this.#socket !== socket) return;
      // On a dead TCP connection onclose can take minutes; don't wait for it.
      socket.onclose = null;
      socket.close();
      lost();
    }, ms);
  }

  async #lost(socket: WebSocket, opened: boolean) {
    if (this.#socket !== socket) return; // replaced by a newer connection
    this.#socket = null;
    clearTimeout(this.#watchdog);
    // A rejected upgrade looks like any other failure to the WebSocket API; ask over HTTP.
    const status = opened ? "offline" : await this.#probe();
    // The proxy reports what the socket can't, e.g. the Frame's key not matching the pairing.
    this.error = await invoke<string | null>("last_error").catch(() => null);
    if (this.#socket || this.#retry !== undefined) return; // reconnected meanwhile
    this.status = status;
    this.#retry = setTimeout(() => {
      this.#retry = undefined;
      this.connect();
    }, RETRY_MS);
  }

  /** Wrong token (401) vs. unreachable; needs CORS on /api/state. */
  async #probe(): Promise<ConnectionStatus> {
    try {
      const url = `http://${this.authority}/api/state?${this.#query()}`;
      const response = await fetch(url, { signal: AbortSignal.timeout(RETRY_MS) });
      return response.status === 401 ? "unauthorized" : "offline";
    } catch {
      return "offline";
    }
  }

  #close() {
    clearTimeout(this.#watchdog);
    clearTimeout(this.#retry);
    this.#retry = undefined;
    const socket = this.#socket;
    this.#socket = null;
    socket?.close();
  }
}

export const agent = new Agent();

/** Android 17+ blocks the LAN until "Nearby devices" is granted; fails like a timeout. */
export function localNetworkBlocked() {
  return window.FrameMateAndroid?.localNetworkAllowed?.() === false;
}
