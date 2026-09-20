// The two ways the frontend reaches the backend.
//
// Inside the Tauri window, commands go through the `call` command and state
// arrives as the `state-updated` event. Opened as a plain browser tab (with
// `pnpm run dev`), `window.__TAURI_INTERNALS__` doesn't exist, so the same
// calls go to the dev bridge instead - the real backend behind an HTTP +
// WebSocket server (`blockloom-app --features dev-bridge`). Both paths end in
// `Backend::dispatch`, so a new command needs no work here.
import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { listen as tauriListen, type Event } from '@tauri-apps/api/event';
import { getVersion as tauriGetVersion } from '@tauri-apps/api/app';

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

const BRIDGE_ORIGIN = 'http://127.0.0.1:4128';
const BRIDGE_WS = 'ws://127.0.0.1:4128/events';

/** Commands the window process handles itself; everything else is backend. */
const WINDOW_COMMANDS = new Set(['reset_zoom', 'pick_project_file', 'set_theme_background']);

function windowInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (WINDOW_COMMANDS.has(cmd)) return tauriInvoke<T>(cmd, args);
  return tauriInvoke<T>('call', { cmd, args });
}

async function bridgeInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  // No native dialogs in a browser tab - ask for the path instead.
  if (cmd === 'pick_project_file') {
    return (window.prompt('Path to the .blockloom file', (args.defaultName as string) ?? '') || null) as T;
  }
  if (cmd === 'reset_zoom' || cmd === 'set_theme_background') return undefined as T;
  const res = await fetch(`${BRIDGE_ORIGIN}/invoke/${cmd}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(args),
  });
  const body = await res.json();
  if (!body.ok) throw new Error(body.error ?? `dev bridge: ${cmd} failed`);
  return body.data as T;
}

type StateListener = (payload: unknown) => void;
const stateListeners = new Set<StateListener>();
let socket: WebSocket | null = null;

function ensureSocket() {
  if (socket) return;
  socket = new WebSocket(BRIDGE_WS);
  socket.onmessage = evt => {
    const payload = JSON.parse(evt.data);
    stateListeners.forEach(cb => cb(payload));
  };
  socket.onclose = () => {
    socket = null;
    if (stateListeners.size > 0) setTimeout(ensureSocket, 1000);
  };
  socket.onerror = () => socket?.close();
}

async function bridgeListen<T>(event: string, cb: (evt: Event<T>) => void): Promise<() => void> {
  if (event !== 'state-updated') {
    console.warn(`dev bridge: unsupported event "${event}"`);
    return () => {};
  }
  ensureSocket();
  const wrapped: StateListener = payload => cb({ event, id: 0, payload: payload as T });
  stateListeners.add(wrapped);
  return () => stateListeners.delete(wrapped);
}

export const invoke = isTauri ? windowInvoke : bridgeInvoke;
export const listen = isTauri ? tauriListen : bridgeListen;
export const getVersion = isTauri ? tauriGetVersion : async () => 'dev-bridge';
