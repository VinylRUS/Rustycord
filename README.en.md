# 🦀 RustyCord

**An unofficial Discord client written in pure Rust: a native GPUI interface (the engine behind the Zed editor) on top of a single protocol core.**
The former Tauri + Svelte 5 version is now legacy and updates itself to the new one automatically.
For people tired of a chat app that eats more RAM than their IDE.

[![CI](https://img.shields.io/github/actions/workflow/status/VinylRUS/Rustycord/ci.yml?branch=main&label=CI&logo=github)](https://github.com/VinylRUS/Rustycord/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/VinylRUS/Rustycord?label=release&logo=github)](https://github.com/VinylRUS/Rustycord/releases)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

[🇷🇺 Русский](README.md) · 🇬🇧 English

[Why this exists](#-why-this-exists) · [Numbers](#-numbers) · [Architecture](#-the-core-the-primary-client-and-the-legacy-version) · [What works](#-what-works-and-what-doesnt) · [Getting started](#-getting-started) · [Roadmap](#-where-the-project-is-heading) · [Help wanted](#-help-wanted) · [Community](#-community)

> ⚠️ **Read this before running.** RustyCord uses a **user token** (like Vencord/Vesktop), which formally goes against Discord's rules. We can't predict how Discord will treat a given account, so use the client at your own risk. We **do not help** with selfbots, spam, automation on behalf of an account, or bypassing Discord's protections, and this project has no use for any of that. See the [FAQ](#-faq).

---

## 💡 Why this exists

It started when the official Discord client ate almost 3 GB of RAM. I saw people rebuilding Adobe and Microsoft Office-style apps in Rust with the help of neural networks and thought: "why not me?"

First came RustyCord on Tauri: a client without Electron that starts and lives much lighter. Then in v0.11 the whole protocol moved into a separate core, `rustycord-core`, and a second, **fully native** frontend grew on top of it — built with GPUI, the very engine the Zed editor is written in. A single multi-megabyte binary, instant startup, no web engine at all. As of v0.12.0 the GPUI version is the primary client, and the Tauri version is legacy: its sources stay in the repository, and installed copies offer to update themselves to the new one.

But the project has a goal more interesting than being lightweight: **to add what Discord could have built itself but never did.** Not tools for breaking rules, just everyday quality of life, so that using this client doesn't feel awful.

---

## 📊 Numbers

| | GPUI (primary) | Tauri (legacy) | Official Discord |
|---|---|---|---|
| Size | a **5–9 MB** binary | installer 4–12 MB (AppImage is 93 MB — it bundles everything) | hundreds of megabytes |
| RAM | native app, no web engine — **send us your measurements** | **~83 MB** at peak¹ | **~500 MB** right after launch |

¹ Author's measurement: Windows 11, Task Manager, all WebView2 processes + core, peak while browsing large servers. Sizes are from the v0.11.0 release archives. One machine, not a lab benchmark. If you get different numbers, please send them in an issue.

---

## 🏗 The core, the primary client and the legacy version

A Cargo workspace with three parts:

- **`crates/rustycord-core`** — the entire Discord protocol with zero UI dependencies: Gateway (WebSocket), REST, voice (songbird + cpal), remote-auth for QR login, and a client fingerprint 1:1 with the official one (X-Super-Properties, IDENTIFY). The UI contract is two traits: `EventSink` (events) and `CaptchaSolver` (captcha).
- **`app-gpui`** — the primary client on GPUI 0.2: one binary, IME input (Russian works), native file dialogs, installers for every platform. It needs testers the most.
- **`src-tauri`** — the **legacy** version on Tauri 2 + Svelte 5: the system WebView instead of Chromium, threads/forums and JS plugins. Not shipped in releases since 0.12.0; installed copies migrate to GPUI automatically.

---

## 🧭 What works and what doesn't

We are honest about the state of the project. It's early: alive, but rough.

| Area | GPUI (primary) | Tauri (legacy) |
|---|---|---|
| Login: token (user/bot), email+password → 2FA, QR | ✅ | ✅ |
| Captcha (a separate discord.com window) | ✅ (except macOS — nuance below) | ✅ |
| Chat: messages, reactions, attachments, edit/delete | ✅ | ✅ |
| DMs, groups, friends with statuses | ✅ | ✅ |
| Threads and forums | ❌ for now | ✅ |
| Voice: channels, mute/deafen, speaking indicators | ✅ | ✅ |
| Noise suppression + AGC, mic test | ✅ | ✅ |
| Go Live and camera: signaling, stream chips, local preview | ✅ | ✅ |
| Sending video to viewers (RTP pipeline) | ❌ goal of the next release | ❌ goal of the next release |
| Moderation: kick/ban from the member list | ✅ | ✅ |
| Vencord-style JS plugins | ❌ (Silent Typing and Message Logger are native toggles) | ✅ |
| Auto-update | manual check; a built-in updater is planned | ✅ migrates to GPUI automatically |
| Token storage | in-memory only — nothing stays on disk | WebView `localStorage` |
| Look and feel | 🐞 early | 🐞 lots of bugs, needs a full pass by a tester |

**Platforms.** GPUI: Windows x64, Linux x64, macOS universal (Apple Silicon + Intel). Tauri (legacy): Windows, Linux, macOS universal. Only the Tauri version has been verified by hand, on **Windows 11** — everything else builds in CI but has never actually been run. If you're on another system, you're gold to us 👇

---

## 🚀 Getting started

### Prebuilt binaries

The [Releases](https://github.com/VinylRUS/Rustycord/releases) page carries the GPUI client:

- **installers:** `RustyCord_<v>_x64-setup.exe` (Windows, NSIS — same as the former Tauri one), `RustyCord_<v>_universal.dmg` (macOS, Apple Silicon + Intel), `RustyCord_<v>_amd64.AppImage` and `rustycord_<v>_amd64.deb` (Linux);
- **portable archives:** `rustycord-gpui-windows-x64.zip`, `rustycord-gpui-linux-x64.tar.gz`, `rustycord-gpui-macos-universal.tar.gz` — just a binary, no installation.

An installed Tauri version will offer to migrate to GPUI in its update card ("Update automatically"): the installer lands in the same place, shortcuts and the entry in "Add/Remove Programs" are preserved. deb/rpm users on Linux should install the new `.deb`/`.AppImage` manually. The last Tauri installers are in releases ≤ 0.11.0.

### From source

You need [Rust](https://rustup.rs) ≥ 1.82.

**GPUI version** (native):

```bash
git clone https://github.com/VinylRUS/Rustycord.git && cd Rustycord
cargo run -p rustycord-gpui --release
```

Linux dependencies (webkit is only needed for the captcha and preview windows): `sudo apt install libwebkit2gtk-4.1-dev libasound2-dev libopus-dev cmake libfontconfig1-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev`.
Even without system libraries the code stays checkable: `cargo check -p rustycord-gpui --no-default-features` (headless mode, same as CI).

**Tauri version** (also needs [Node.js](https://nodejs.org) ≥ 20 and the Tauri system libraries, [list per OS](CONTRIBUTING.en.md#building-and-running)):

```bash
npm install
npm run tauri dev      # opens the app window
```

Demo of the Tauri version without logging in: `npm run dev`, then open `http://localhost:1420`.

Verbose logs for either version: `RUST_LOG=debug` (e.g. `RUST_LOG=debug cargo run -p rustycord-gpui`).

---

## 🛠 What's inside

- **Core `rustycord-core`:** tokio, tokio-tungstenite (Gateway), reqwest (REST), songbird (voice), cpal (audio devices); remote-auth, client fingerprint. The protocol write-up (Gateway, op 37, voice, remote-auth) lives in the detailed code comments of the core — if reverse engineering is your thing, start there.
- **Native UI (`app-gpui`):** GPUI 0.2, an IME composer (EntityInputHandler), rfd file dialogs, captcha and preview windows on wry + winit; installer packaging lives in `packaging/`.
- **Classic UI (`src-tauri`, legacy):** Tauri 2, Svelte 5 + TypeScript, the system WebView instead of Chromium; not shipped in releases, updates users to GPUI.
- **Plugins:** Vencord-style JS files for the Tauri version (see [`plugins/`](plugins)); in GPUI their features are native settings toggles.

---

## 🔭 Where the project is heading

The official Discord client is moving to Rust too — so the idea wasn't so crazy after all. Our niche doesn't change: lightweight, honest about its state, comfortable. Nearest plans:

- **A video pipeline for Go Live and the camera** (capture → codec → RTP over the voice UDP socket): right now both versions handle signaling and local preview, but the picture can't reach viewers yet — that's the main goal of the next major release.
- **GPUI version: persisting the token and settings across runs** (today the token lives only in session memory), threads and forums, a built-in updater (for now updates are manual via the release installers).
- **QoL features Discord lacks:** pinned chats on top of the list, a single unread list across all servers, custom channel order and grouping, local history search. The yardstick is simple: a feature improves a person's life instead of doing something on their behalf.
- **Bot panels inside the client (concept).** A bot publishes a **declarative manifest** (a description of panels and buttons, no executable code) and the client renders the UI itself. Still a sketch — discuss it in [Discussions](https://github.com/VinylRUS/Rustycord/discussions).

---

## 🙋 Help wanted

The project has no money and I can't pay anyone. What I can offer: **a big human thank-you**, your name in the [CHANGELOG](CHANGELOG.md) and the contributors list, and careful review. My time is short (my day job eats nearly all of it), so I usually reply every one to two weeks.

Pick your door:

| If you… | You can help with |
|---|---|
| 🦀 **write Rust by hand**, not just prompt an AI | The RTP video pipeline, voice and songbird, the GPUI frontend, the Gateway. The most needed help. |
| 🧪 **are a tester / QA** (no code required) | Walk through both versions and write bugs down as a list with steps and logs. We badly need a dedicated person for this. |
| 🎨 **are into native UI or design** | The GPUI version's look and feel is early — it needs a designer's pass. |
| 🐧 **use Linux or macOS** | Run the GPUI binary and tell us what breaks. Native rendering, audio and captcha live there — surprises are almost guaranteed. |
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

**Which one do I download?** The GPUI version — as of 0.12.0 it's the primary client (native, lightweight, no token left on disk). The Tauri version is legacy: it's no longer shipped in releases, and installed copies offer to update themselves to GPUI. It only makes sense to run the legacy version if you need threads, forums or JS plugins right now — build it from source or grab a release ≤ 0.11.0.

**I have the Tauri version installed — how do I migrate?** Nothing special to do: click "Update automatically" in the update card — the client downloads the GPUI installer and puts it in place (Windows/macOS/AppImage; deb/rpm users on Linux should download the `.deb`/`.AppImage` from the release manually). On first launch, log in again with a QR code or token: the legacy version kept it in the WebView's `localStorage`, and it doesn't carry over.

**Is this legal?** User-token clients (Vencord, Vesktop and others) have existed for a long time, but formally this is against Discord's rules. Bans for ordinary use are rare but possible. The decision is yours.

**Is it safe to enter my token?** The token is only ever sent to Discord's servers. Beyond that it depends on the version: **GPUI** keeps it in memory only and nothing stays on disk after you close the app (which is why you re-enter it every launch); **Tauri** stores it in the WebView's `localStorage`. Moving it into the OS credential store (Keychain/Credential Manager/libsecret) is a great task for a contributor. Don't paste your token into other programs and don't share logs that contain it.

**Why a user token instead of a bot?** Because we want the full client experience: servers, DMs, voice, not just what a bot can do. Bot mode remains for debugging.

**Discord asked for a captcha.** That's anti-fraud. RustyCord opens a separate confirmation window on discord.com; solve it there and login continues. One nuance: in the GPUI version on macOS the captcha window is not available yet — use token or QR login there.

**I started Go Live and nobody can see me.** A known limitation: signaling, stream chips and local preview work, but the video pipeline to viewers (capture → codec → RTP) is still in development — it's the main goal of the next major release. We say so honestly in the table above.

---

## 📄 License and disclaimer

MIT, see [LICENSE](LICENSE). The project is not affiliated with Discord Inc.; Discord is a trademark of Discord Inc. The authors are not responsible for account bans. Please don't hurt the crabs while building. 🦀
