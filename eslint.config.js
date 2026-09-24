// @ts-check
import js from "@eslint/js";
import tseslint from "typescript-eslint";
import svelte from "eslint-plugin-svelte";
import svelteParser from "svelte-eslint-parser";
import globals from "globals";

export default tseslint.config(
  {
    ignores: [
      "build/",
      ".svelte-kit/",
      "node_modules/",
      "coverage/",
      "src-tauri/",
      "src/lib/bindings.ts",
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  ...svelte.configs.recommended,
  {
    languageOptions: {
      globals: { ...globals.browser, ...globals.node },
    },
  },
  {
    files: ["**/*.svelte"],
    languageOptions: {
      parser: svelteParser,
      parserOptions: {
        parser: tseslint.parser,
        extraFileExtensions: [".svelte"],
      },
    },
  },
  {
    // `*.svelte.ts` are plain TS modules (Svelte 5 runes outside a
    // component); the svelte plugin's recommended config otherwise routes
    // them through `svelte-eslint-parser`, which does not understand
    // TS-only syntax like `import type { ... }`.
    files: ["**/*.svelte.ts"],
    languageOptions: {
      parser: tseslint.parser,
    },
  },
  {
    files: ["src/lib/mock.js"],
    languageOptions: {
      sourceType: "script",
      globals: { ...globals.browser },
    },
  },
  {
    rules: {
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_" }],
      "no-unused-vars": "off",
      // These `Map`s are deliberately plain: the store recomputes them
      // wholesale (`store.recompute()`, coalesced to one pass per animation
      // frame — see store.svelte.ts) rather than tracking per-entry
      // mutations, so per-item reactivity would be wasted work, not a bug.
      "svelte/prefer-svelte-reactivity": "off",
    },
  },
);
