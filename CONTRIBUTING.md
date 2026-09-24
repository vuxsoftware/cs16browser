# Contributing to cs16browser

## Prerequisites

- [Bun](https://bun.sh/)
- A Rust toolchain
- The [Tauri 2 system prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform

## Setup

```sh
bun install --frozen-lockfile
bun run tauri dev
```

Enter the Steam Web API key in the app's Settings dialog. The app does not read `STEAM_API_KEY` or `.env`.

For the architecture, module layout, and the implementation rules the codebase relies on, see [AGENTS.md](AGENTS.md).

## Quality checks

```sh
bun run quality:frontend
bun run build
cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo llvm-cov --lib --tests --fail-under-lines 80 --summary-only
```

The frontend gate runs Svelte checks, dead-code analysis, Vitest with an 80% line-coverage threshold, and version consistency checks. CI also runs the Rust checks and tests.

## Project website

The static website lives in [`docs/`](docs/index.html). Its source is split per section under [`docs/src/`](docs/src/) (`sections/*.html`, `styles/*.css`, `site.js`); run `bun run site` to assemble it into `docs/index.html`, `docs/style.css` and `docs/site.js`, and commit the result. `bun run site:check` fails when the generated files are stale. Its main image is a copy of [`assets/app.png`](assets/app.png); update `docs/assets/app.png` when the app screenshot changes. To preview the site locally, open `docs/index.html` in a browser.

The repository slug (`vuxsoftware/cs16browser`) is set once in `SITE` in `scripts/build-site.mjs`. From it, `site.js` fetches the star count and, once a release is published, points the Download buttons at the Windows installer and shows its version. If the GitHub API is unavailable, the buttons open the releases page and the badge shows `—`.

To publish it with GitHub Pages:

1. Open **Settings → Pages** in the repository.
2. Under **Build and deployment**, select **Deploy from a branch**, choose the default branch and **`/docs`**, then save.
3. After the Pages deployment finishes, open the site at `https://<owner>.github.io/<repository>/`. GitHub also shows the exact URL in **Settings → Pages**.

Changes pushed to `docs/` on that branch update the site automatically. The site is only a static description of the desktop app; the server discovery and UDP scanner run in the installed app. See [GitHub's publishing-source guide](https://docs.github.com/en/pages/getting-started-with-github-pages/configuring-a-publishing-source-for-your-github-pages-site) for Pages setup and troubleshooting.

## Publishing signed releases

The updater endpoint in `src-tauri/tauri.conf.json` points at `github.com/vuxsoftware/cs16browser`. The updater signing key is separate from a Windows code-signing certificate. Before a release:

1. Back up the private updater key outside the repository (`~/.tauri/cs16browser.key` on the maintainer's machine). Its matching public key is in `tauri.conf.json`. Losing the private key prevents updates to builds that trust this public key.
2. Add the **contents** of the private key as the `TAURI_SIGNING_PRIVATE_KEY` GitHub Actions repository secret. A path to the maintainer's local file will not exist on the GitHub runner. This key has no password, so `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` is not needed.
3. Run `bun run version:set X.Y.Z` to update the frontend, Tauri, Rust crate, lockfile and native window title together. Push the release commit to `main`, then push a matching `vX.Y.Z` tag on that commit. A push to `main` runs quality checks; only the tag starts the installer and GitHub Release workflow.
4. After CI publishes the release, install the NSIS bundle on a clean Windows 10 or 11 machine and verify first-run key setup, server discovery, Connect, and a manual update check. Check that the website Download button selects the installer.

The release workflow builds the Windows NSIS installer, signs its updater artifact, and publishes `latest.json`. Installed builds verify signatures before applying an update. The Windows installer itself is not code-signed, so SmartScreen may warn.
