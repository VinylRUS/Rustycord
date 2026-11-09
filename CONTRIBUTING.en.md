# Contributing to RustyCord

[🇷🇺 Русский](CONTRIBUTING.md) · 🇬🇧 English

Thanks for stopping by! RustyCord is built by one person in their spare time, so every extra pair of hands is worth its weight in gold. This document exists so you spend less time guessing and I spend less time asking follow-up questions.

**The short version:**
- There's no money. There's a thank-you, your name in the [CHANGELOG](CHANGELOG.md), and careful review.
- My time is short. I usually reply **every one to two weeks**, sometimes with longer pauses. You're not being ignored; my day job just eats nearly everything.
- Read the [CODE_OF_CONDUCT](CODE_OF_CONDUCT.md): be respectful, no politics, no ethnic slurs.

---

## Repository layout

A Cargo workspace with three parts (details in the [README](README.en.md#-two-frontends-one-core)):

- `crates/rustycord-core` — the entire Discord protocol, no UI dependencies;
- `app-gpui` — the native GPUI frontend;
- `src-tauri` + `src/` — the Tauri 2 + Svelte 5 frontend.

The general rule: **protocol logic goes into the core, UI stays in a frontend.** If a feature is needed by both frontends, it almost certainly belongs in the core.

---

## Pick your role

- 🦀 **Rust developer.** The most needed help: the RTP video pipeline for Go Live/camera, voice and songbird, the GPUI frontend, the Gateway, decoders, bugs the author couldn't solve.
- 🧪 **Tester / QA.** No code required: walk through both versions and write bugs down as a list. We badly need a dedicated person for this.
- 🎨 **Svelte / design.** The Tauri version's look and feel is currently one big bug.
- 🐧 **Linux / macOS.** Just run a build (especially the GPUI binary — it's the youngest) and tell us what happened.
- 🔬 **Protocol reverse engineering.** Cross-checking with the official client, documentation.

---

## What we accept and what we don't

**We accept** whatever improves life for users: bug fixes, stability, convenience, new QoL features. The yardstick is simple: *a feature helps a person instead of doing something on their behalf.*

**We do not accept, and close without discussion:**
- selfbots, mass messaging, automation of actions on behalf of an account;
- bypassing Discord's protections, anti-fraud, captchas or limits;
- any sending of the token or user data anywhere except Discord's servers;
- features whose goal is to hide the client from Discord or pass it off as official.

Simple QoL ideas for the Tauri version are often better as a **plugin** (see [`plugins/`](plugins)) than baked into the core. The GPUI version has no JS plugins — propose the equivalent as a native feature with a settings toggle. If you're not sure where your idea belongs, open a Discussion before writing code.

---

## Building and running

You need [Rust](https://rustup.rs) ≥ 1.82. What else depends on the version you work on.

### GPUI version (native)

```bash
git clone https://github.com/VinylRUS/Rustycord.git && cd Rustycord
cargo run -p rustycord-gpui
```

- **Ubuntu/Debian** (webkit is only needed for the captcha and preview windows): `sudo apt install libwebkit2gtk-4.1-dev libasound2-dev libopus-dev cmake libfontconfig1-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev`
- **macOS:** Xcode Command Line Tools (the captcha window is not available yet — use token/QR login).
- **Windows:** MSVC Build Tools, WebView2 for the captcha window.

Headless check without system libraries (same as CI): `cargo check -p rustycord-gpui --no-default-features`.

### Tauri version

Additionally needs [Node.js](https://nodejs.org) ≥ 20 and the Tauri system libraries:

- **Ubuntu/Debian:** `sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev libopus-dev cmake`
- **macOS:** Xcode Command Line Tools.
- **Windows:** [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) and [MSVC Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/).

```bash
npm install
npm run tauri dev
```

Demo mode without logging in: `npm run dev`, then open `http://localhost:1420`.

### Logs

```bash
# GPUI (Linux / macOS)
RUST_LOG=debug cargo run -p rustycord-gpui

# Tauri (Linux / macOS)
RUST_LOG=debug npm run tauri dev

# Windows (PowerShell)
$env:RUST_LOG="debug"; npm run tauri dev
```

---

## Checks before a PR

CI runs the same commands, so run them locally. A PR with red CI will not be reviewed (see [`.github/workflows/ci.yml`](.github/workflows/ci.yml) for the exact flags).

```bash
# formatting for the whole workspace
cargo fmt --all -- --check

# core + GPUI frontend (headless — no system libraries needed)
cargo clippy -p rustycord-core -p rustycord-gpui --no-default-features --all-targets -- -D warnings
cargo test -p rustycord-core -p rustycord-gpui --no-default-features

# Tauri version
cd src-tauri
cargo clippy --all-targets -- -D warnings
cargo test
cd ..

# Tauri's web frontend
npx svelte-check
npm run build
```

GPUI with full features (captcha window, audio) is checked by a separate CI job on Linux only — if you touched the `captcha-webview` or `audio` features, mention it in the PR.

---

## Pull request rules

1. **One PR, one problem.** Small changes are easy to review and merge; big rewrites sit for months.
2. **Discuss big things first.** For anything beyond a few dozen lines or anything that changes an event format, open an issue or a discussion first.
3. **Don't break consumers.** If you change an event format or an IPC command, check who reads it: the other frontend, the core, the Svelte side.
4. **Add a test** when you fix a bug that a test can reproduce. A regression caught once should stay caught.
5. **Don't swallow errors for the sake of "stability".** Errors must be visible in the log, but **without tokens, cookies or authorization headers**.
6. **Describe what you checked by hand**: which OS, which frontend (GPUI or Tauri), and what exactly you clicked.
7. **No delays, retries or bulk subscriptions "just in case".** Any limit must rest on the protocol and measurements.

### AI-written code

Code written by a neural network is **allowed** when:
- CI is green;
- you understand what's in it and can explain it to a reviewer;
- you've noted in the PR description that you used AI and briefly said what you verified by hand. This isn't a ban, it's a hint about where to look more carefully.

---

## Sensitive areas

Changes here always go through the project owner's review, even from trusted maintainers (see [`.github/CODEOWNERS`](.github/CODEOWNERS)):

- login and token storage (in both frontends);
- the protocol core `crates/rustycord-core`: the REST client, the Gateway, the client fingerprint (X-Super-Properties), voice;
- the captcha and preview windows (`app-gpui`, `src-tauri/src/auth`);
- the plugin system;
- Tauri permissions and configuration (`tauri.conf.json`, capabilities);
- CI workflows and releases.

---

## Where to start

> These are candidates gathered from write-ups and logs. Before starting, check in the current code that the problem still exists. The maintainer will turn the relevant items into issues labeled `good first issue` and `help wanted`.

**Rust**
- The video pipeline for Go Live and the camera (screen/camera capture → codec → RTP over the voice UDP socket) — the top priority of the next major release; signaling already works in both frontends.
- GPUI: persisting the token and settings across runs (today it's session memory only; the format is up for discussion: an encrypted file or the OS credential store).
- GPUI: threads and forums (the Tauri version already has them — borrow the mechanics from there).
- Building the GPUI binary for Intel-macOS (CI only builds arm64).
- Moving the token to the OS credential store (Keychain / Credential Manager / libsecret) for the Tauri version instead of `localStorage`.
- Gateway: reset the backoff after a successful session; a limiter for repeated IDENTIFY after a streak of 4002s; logging the last outgoing frames (without secrets) when a connection closes.
- Member list: a regression test for switching channels inside a guild that has a hidden list.

**Svelte / Tauri frontend**
- Forward frontend errors to the log with rate limiting (first N with a full stack, then a counter) so spam doesn't drown everything else.
- Look and feel: first compile a list of problems, then fix them.

**QoL ideas**
- Pinned chats on top of the list.
- A single unread list across all servers.
- Custom channel order and grouping.
- Local history search.

**Testers**
- The GPUI version on every platform: a full pass, especially voice, captcha and IME input.
- The Tauri version on Linux, macOS or Windows 10: what works out of what CI builds.

---

## How to write a bug report

Use the template when creating an issue. It asks for:
- the RustyCord version, your operating system and **which frontend — GPUI or Tauri**;
- what you did, what you expected and what happened;
- a log captured with `RUST_LOG=debug` (see above).

**Check your log before sending:** remove the token if it ended up there. IDs and session IDs may stay, but if in doubt replace them with `<redacted>`.

---

## Response times

- Replies to issues and PRs: usually **every 1–2 weeks**.
- Issues without activity may be closed after a month. If the problem is still relevant, say so and we'll reopen it.
- Vulnerabilities: **not in public issues**, follow the instructions in [SECURITY.md](SECURITY.md).

---

## Maintainers

Write access goes to people with **several accepted PRs over different periods of time** (not a batch in a single evening) who have shown care, especially in sensitive areas. Safeguards stay in place: a protected `main` branch, mandatory review, CODEOWNERS on critical files, signing keys and CI secrets held only by the owner, and two-factor authentication for everyone with access.

---

## Contribution license

By submitting a PR you agree that your code will be distributed under the project's license ([MIT](LICENSE)).
