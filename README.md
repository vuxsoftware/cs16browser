# cs16browser

**Find real Counter-Strike 1.6 servers. Skip suspicious listings.**

cs16browser is a free, open-source desktop server browser for Counter-Strike 1.6. It fetches Steam's server list, checks servers directly with the GoldSrc A2S protocol, and highlights listings that look fake, redirected, or spammed.

> **Free to use · Open source · No account in the app · Steam Web API key stored locally**

---

## 🎮 Why use it?

The Steam listing can contain redirect farms and other misleading entries. cs16browser checks what servers actually report before showing them in the Internet list. You can filter the results locally, save favorites, and launch a server through Steam.

| | Feature | What it does |
|---|---|---|
| 🔎 | Direct server checks | Queries listed endpoints over UDP for their name, map, players, latency, and other details. |
| 🛡️ | Suspicious-server detection | Flags fake, redirect, and spam signals; confirmed bad entries stay out of the normal Internet view. |
| ⚡ | Local filters | Narrow the list by a case-insensitive part of the server name or map, latency, anti-cheat status, and player conditions without fetching Steam again. |
| ⭐ | Favorites and history | Keep servers you care about and find previously visited ones. |
| 🔄 | Two refresh choices | **Quick refresh** queries visible servers; **Refresh all** fetches a fresh list from Steam. Query pacing avoids repeatedly hitting the same endpoint. |
| 🔐 | Signed updates | Choose whether to check at startup, check manually in Settings, and decide when to install. |

---

## 🚀 Getting started

1. **Build and open the app.** Release downloads will be linked here after the first public release. For now, use the development steps below.
2. **Add a Steam Web API key** in Settings. The app opens Settings on first run and links to Steam's [API key page](https://steamcommunity.com/dev/apikey). The key is stored locally and is never returned to the webview.
3. **Browse and filter.** The Internet tab shows servers that answered the direct check and passed the browser's verification. Use **Name** to match any part of a server name, regardless of letter case. Use Map, Latency, Anti-cheat, and the checkboxes to narrow results further.
4. **Refresh when needed.** **Quick refresh** rechecks the servers currently shown. **Refresh all** asks Steam for a fresh server listing, then scans it. The per-server pacing rules can defer a query that happened too recently.
5. **Connect.** Select a server and press **Connect** to open `steam://connect/<ip>:<port>/` through Steam.

**Privacy:** the Steam Web API key lives in `cs16browser-data/settings.json` on your computer and is sent to Steam when requesting the server list. Direct server checks send UDP queries to server endpoints. Favorites, history, filters, and the automatic update-check preference are stored locally.

---

## 💻 Platforms

cs16browser supports **Windows 10 and 11 (64-bit)**. Releases ship as an NSIS installer. Counter-Strike 1.6 and Steam are needed to join a server; browsing and verification are performed by this app.

---

## 🌐 Project website

The static website lives in [`docs/`](docs/index.html). Its source is split per section under [`docs/src/`](docs/src/) (`sections/*.html`, `styles/*.css`, `site.js`); run `bun run site` to assemble it into `docs/index.html`, `docs/style.css` and `docs/site.js`, and commit the result. `bun run site:check` fails when the generated files are stale. Its main image is a copy of [`assets/app.png`](assets/app.png); update `docs/assets/app.png` when the app screenshot changes. To preview the site locally, open `docs/index.html` in a browser.

To publish it from this repository with GitHub Pages:

1. Push the repository to GitHub. On GitHub Free, the repository must be public for Pages.
2. Open **Settings → Pages** in the repository.
3. Under **Build and deployment**, select **Deploy from a branch**, choose the default branch and **`/docs`**, then save.
4. After the Pages deployment finishes, open the site at `https://<owner>.github.io/<repository>/`. GitHub also shows the exact URL in **Settings → Pages**.

Changes pushed to `docs/` on that branch update the site automatically. The site is only a static description of the desktop app; the server discovery and UDP scanner run in the installed app. See [GitHub’s publishing-source guide](https://docs.github.com/en/pages/getting-started-with-github-pages/configuring-a-publishing-source-for-your-github-pages-site) for Pages setup and troubleshooting.

The repository slug (`vuxsoftware/cs16browser`) is set once in `SITE` in `scripts/build-site.mjs`. From it, `site.js` fetches the star count and, once a release is published, points the Download buttons at the Windows installer and shows its version. If the GitHub API is unavailable, the buttons open the releases page and the badge shows `—`.

---

## 🛠️ Develop

You need [Bun](https://bun.sh/), a Rust toolchain, and the [Tauri 2 system prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform.

```sh
bun install --frozen-lockfile
bun run tauri dev
```

Enter the Steam Web API key in the app's Settings dialog. The app does not read `STEAM_API_KEY` or `.env`.

### Quality checks

```sh
bun run quality:frontend
bun run build
cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo llvm-cov --lib --tests --fail-under-lines 80 --summary-only
```

The frontend gate runs Svelte checks, dead-code analysis, Vitest with an 80% line-coverage threshold, and version consistency checks. CI also runs the Rust checks and tests. For implementation rules and protocol notes, see [AGENTS.md](AGENTS.md).

<details>
<summary>Publishing signed releases</summary>

The updater endpoint in `src-tauri/tauri.conf.json` still uses a placeholder GitHub owner. Before the first release:

1. Create the public repository and replace the placeholder release URL with its actual URL.
2. Add the private updater signing key as the `TAURI_SIGNING_PRIVATE_KEY` GitHub Actions secret. Keep a secure backup outside the repository. The matching public key is already in `tauri.conf.json`.
3. Keep versions in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json` aligned, then push a matching `vX.Y.Z` tag.

The release workflow checks these requirements, builds bundles, signs updater artifacts, and publishes `latest.json`. Installed builds verify signatures before applying an update. Operating-system code signing is separate from Tauri's updater signing.

</details>

---

## 🤝 Project

Made by **cs16browser contributors**. The app is free and intended to stay free. Contributions and issue reports are welcome once the public repository is configured. The project is licensed under the [GNU GPL v3](LICENSE) (or, at your option, any later version).

**Unofficial project.** cs16browser is not affiliated with, sponsored by, or endorsed by Valve Corporation. Counter-Strike and Steam are trademarks of Valve Corporation.
