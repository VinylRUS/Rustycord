# 🦀 RustyCord

**A lightweight unofficial Discord client built with Rust + Tauri + Svelte 5.**
For people tired of a chat app that eats more RAM than their IDE.

[![CI](https://img.shields.io/github/actions/workflow/status/VinylRUS/Rustycord/ci.yml?branch=main&label=CI&logo=github)](https://github.com/VinylRUS/Rustycord/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/VinylRUS/Rustycord?label=release&logo=github)](https://github.com/VinylRUS/Rustycord/releases)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

[🇷🇺 Русский](README.md) · 🇬🇧 English

> ⚠️ **Read this before running.** RustyCord uses a **user token** (like Vencord/Vesktop), which formally goes against Discord's rules. We can't predict how Discord will treat a given account, so use the client at your own risk. We **do not help** with selfbots, spam, automation on behalf of an account, or bypassing Discord's protections, and this project has no use for any of that. See the [FAQ](#-faq).

---

## 💡 Why this exists

It started when the official Discord client ate almost 3 GB of RAM. I saw people rebuilding Adobe and Microsoft Office-style apps in Rust with the help of neural networks and thought: "why not me?"

RustyCord is a client without Electron that starts and lives much lighter. But the project has a second goal that is more interesting than the first: **to add what Discord could have built itself but never did.** Not tools for breaking rules, just everyday quality of life, so that using this client doesn't feel awful.

---

## 📊 Numbers

| | RustyCord | Official Discord |
|---|---|---|
| RAM | **~83 MB** at peak (all WebView2 processes + core; `rustycord.exe` alone is 20–30 MB) | **~500 MB** right after launch |

*Author's measurements: Windows 11, Task Manager, peak while browsing large servers. One machine, not a lab benchmark. If you get different numbers, please send them in an issue.*

---

## 🧭 What works and what doesn't

We are honest about the state of the project. It's early: alive, but rough.

| Area | Status |
|---|---|
| Login (QR, email+password, token) | ✅ works |
| Chat: messages, reactions, attachments | ✅ works |
| Group chats and DMs | ✅ works |
| Threads and forums | ✅ works |
| Voice calls | 🟡 work, stability and microphone transmission need verification |
| Member list and presence | 🟡 work, testers needed across many servers |
| Streaming and screen share | ❌ not working |
| Look and feel | 🐞 lots of bugs, needs a full pass by a tester |

**Platforms.** Verified by hand only on **Windows 11**. Windows 10, Linux and macOS build in CI, but nobody has actually run them yet. If you're on one of those, you're gold to us 👇

---

## 🚀 Getting started

### Prebuilt binaries

Download an installer from [Releases](https://github.com/VinylRUS/Rustycord/releases). *(If the page is empty, builds are not published for every release yet, so build from source.)*

### From source

You need [Node.js](https://nodejs.org) ≥ 20, [Rust](https://rustup.rs) ≥ 1.77 and the Tauri system libraries ([list per OS](CONTRIBUTING.en.md#building-and-running)).

```bash
git clone https://github.com/VinylRUS/Rustycord.git && cd Rustycord
npm install
npm run tauri dev      # opens the app window
```

Demo without logging in: `npm run dev`, then open `http://localhost:1420`.

---

## 🛠 What's inside

- **Core:** Rust, tokio, tokio-tungstenite (Gateway), reqwest (REST), songbird (voice), cpal (audio devices).
- **Shell:** Tauri 2, the system WebView instead of Chromium.
- **UI:** Svelte 5 + TypeScript.
- **Plugins:** Vencord-style JS files (see [`plugins/`](plugins)).

The protocol write-up (Gateway, op 37, voice, remote-auth) lives in the repository: if reverse engineering is your thing, start there.

---

## 🔭 Where the project is heading

We want to grow **wider**, not just catch up with the official client:

- **QoL features Discord lacks.** Starter ideas: pinned chats on top of the list, a single unread list across all servers, custom channel order and grouping, local history search. The yardstick is simple: a feature improves a person's life instead of doing something on their behalf.
- **Bot panels inside the client (concept).** Imagine a movie-night bot: add it to a server and the client shows a panel with its features, not only slash commands. The idea is for a bot to publish a **declarative manifest** (a description of panels and buttons, no executable code) and for the client to render the UI itself. For now it's a sketch, so discuss it in [Discussions](https://github.com/VinylRUS/Rustycord/discussions).
- **Stable voice and video, streaming, a decent look and feel.**

---

## 🙋 Help wanted

The project has no money and I can't pay anyone. What I can offer: **a big human thank-you**, your name in the [CHANGELOG](CHANGELOG.md) and the contributors list, and careful review. My time is short (my day job eats nearly all of it), so I usually reply every one to two weeks.

Pick your door:

| If you… | You can help with |
|---|---|
| 🦀 **write Rust by hand**, not just prompt an AI | Voice and songbird, the Gateway, decoders, bugs the author couldn't solve. The most needed help. |
| 🧪 **are a tester / QA** (no code required) | Walk through the app and write bugs down as a list with steps and logs. We badly need a dedicated person for this. |
| 🎨 **know Svelte or love design** | The look and feel is currently one big bug. |
| 🐧 **use Linux or macOS** | Just run a build and tell us what breaks. There's WebKitGTK and different audio there, so a surprise is almost guaranteed. |
| 🔬 **love protocol reverse engineering** | Cross-checking against the official client, documentation, new findings. |

See [CONTRIBUTING.en.md](CONTRIBUTING.en.md) to get started. AI-written code is allowed as long as CI is green and the author understands what they submit.

---

## 💬 Community

- Discord server: _[link coming soon]_
- Telegram: _[link coming soon]_
- Discussions, ideas, questions: [GitHub Discussions](https://github.com/VinylRUS/Rustycord/discussions)
- Bugs: [Issues](https://github.com/VinylRUS/Rustycord/issues)
- Vulnerabilities: **not in public issues**, see [SECURITY.md](SECURITY.md)

House rules: [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md). In short: be respectful, no politics, no ethnic slurs.

---

## 🧭 Project principles

1. The client exists to **improve** life for Discord users, not to harm the community.
2. No selfbots, mass messaging, automation on behalf of an account, or bypassing Discord's protections. Requests for such things are closed without discussion.
3. No sending the token or user data anywhere except Discord's servers. A PR that does so is rejected immediately.
4. We are honest about the project's state: what works and what's broken is written above.

---

## ❓ FAQ

**Is this legal?** User-token clients (Vencord, Vesktop and others) have existed for a long time, but formally this is against Discord's rules. Bans for ordinary use are rare but possible. The decision is yours.

**Is it safe to enter my token?** The token is stored locally (currently in the WebView's `localStorage`) and is sent only to Discord's servers. Moving it into the OS credential store (Keychain/Credential Manager/libsecret) is a great task for a contributor. Don't paste your token into other programs and don't share logs that contain it.

**Why a user token instead of a bot?** Because we want the full client experience: servers, DMs, voice, not just what a bot can do. Bot mode remains for debugging.

**Discord asked for a captcha.** That's anti-fraud. RustyCord opens a confirmation window on discord.com; solve it there and login continues.

---

## 📄 License and disclaimer

MIT, see [LICENSE](LICENSE). The project is not affiliated with Discord Inc.; Discord is a trademark of Discord Inc. The authors are not responsible for account bans. Please don't hurt the crabs while building. 🦀
