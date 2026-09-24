# cs16browser

**Find real Counter-Strike 1.6 servers. Skip suspicious listings.**

cs16browser is a free, open-source desktop server browser for Counter-Strike 1.6. Steam's server list is flooded with fake and redirect servers registered by a few redirect farms; cs16browser checks servers directly with the GoldSrc A2S protocol before showing them, so the list only has servers you can actually play on.

> **Free to use · Open source · No account in the app · Steam Web API key stored locally**

---

## 🎮 Why use it?

| | Feature | What it does |
|---|---|---|
| 🔎 | Direct server checks | Queries listed endpoints over UDP for their name, map, players, latency, and other details — not just what they told Steam. |
| 🛡️ | Suspicious-server detection | Flags fake, redirect, and spam signals; confirmed bad entries stay out of the normal Internet view. The Banned tab lists everything removed, and one click clears the bans if you trust them. |
| ⚡ | Local filters | Narrow the list by a case-insensitive part of the server name or map, latency, anti-cheat status, bots, and player conditions without fetching Steam again. |
| ☰ | Player info | Open a server's info to see its players with their scores and time on the server. |
| ⭐ | Favorites and history | Keep servers you care about and find previously visited ones. |
| 🔄 | Two refresh choices | **Quick refresh** queries visible servers; **Refresh all** fetches a fresh list from Steam. Query pacing avoids repeatedly hitting the same endpoint. |
| 🔐 | Signed updates | Choose whether to check at startup, check manually in Settings, and decide when to install. |
| ▣ | Familiar interface | Matches the in-game GoldSrc browser down to the pixel: same tabs, same columns, same olive green. |

---

## 🚀 Getting started

1. **Download and install.** Grab the latest Windows installer from the [Releases page](https://github.com/vuxsoftware/cs16browser/releases/latest). If Windows shows a SmartScreen warning, click **More info**, then **Run anyway** — the installer isn't signed with a paid code-signing certificate yet, and the source code is public.
2. **Add a Steam Web API key** in Settings. The app opens Settings on first run and links to Steam's [API key page](https://steamcommunity.com/dev/apikey). Getting one is free: sign in, enter any domain name (`localhost` is fine) and copy the key. The key is stored locally and is never returned to the webview.
3. **Browse and filter.** The Internet tab shows servers that answered the direct check and passed the browser's verification. Use **Name** to match any part of a server name, regardless of letter case. Use Map, Latency, Anti-cheat, and the checkboxes to narrow results further.
4. **Refresh when needed.** **Quick refresh** rechecks the servers currently shown. **Refresh all** asks Steam for a fresh server listing, then scans it. The per-server pacing rules can defer a query that happened too recently.
5. **Connect.** Select a server and press **Connect** to open `steam://connect/<ip>:<port>/` through Steam.

**Privacy:** the Steam Web API key lives in `cs16browser-data/settings.json` on your computer and is sent only to Steam, to fetch the server list. Direct server checks send UDP queries to server endpoints, and the app contacts GitHub to look for updates (the update check can be turned off in Settings). Favorites, history, filters, and the update-check preference are stored locally. There's no account and no tracking.

---

## 💻 Platforms

cs16browser supports **Windows 10 and 11 (64-bit)**. Releases ship as an NSIS installer. Counter-Strike 1.6 and Steam are needed to join a server; browsing and verification are performed by this app.

---

## ❓ FAQ

**Do I need Counter-Strike 1.6 installed?** You can browse without it. To join a server you need Steam and Counter-Strike 1.6 in your library, because joining goes through Steam, just like it does from the game's own browser.

**Won't all those checks flood the servers?** No. If a server answered in the last minute, the app reuses that answer. No server gets more than three queries a minute, and servers that keep timing out are left alone for a while.

More questions are answered on the [project website FAQ](docs/index.html#faq).

---

## 🤝 Project

Made by **vuxsoftware**. The app is free and intended to stay free. Contributions and issue reports are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, test, and develop cs16browser. The project is licensed under the [GNU GPL v3](LICENSE) (or, at your option, any later version).

**Unofficial project.** cs16browser is not affiliated with, sponsored by, or endorsed by Valve Corporation. Counter-Strike and Steam are trademarks of Valve Corporation.
