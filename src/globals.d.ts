/// <reference types="vite/client" />

/* globals.d.ts — ambient declarations for the two script-level seams.
 *
 * `mock.js` is a classic script (loaded for its side effect by `cs16.ts`, so
 * the standalone review build works), so its global is declared here rather
 * than exported from a module.
 */

import type { Event, ServerRow } from "./lib/bindings";
import type * as store from "./lib/store.svelte";
import type { UiState } from "./lib/store.svelte";
import type { Backend } from "./lib/cs16";

declare global {
  interface Window {
    /** The scripted fixture/replay, present only outside Tauri. */
    cs16Mock?: Backend;
    /** Pushes synthetic backend events through the real render path, so a
     * 20 000-row sweep can be checked without standing up the Rust backend.
     * It exposes no state that changes behaviour. */
    cs16Debug?: {
      feed(event: Event): void;
      state: UiState;
      store: typeof store;
    };
  }
}

/** Kept so this file is a module (required for `declare global` to augment
 * rather than replace). */
export type { Event, ServerRow };
