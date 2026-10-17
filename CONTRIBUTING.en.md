# Contributing to RustyCord

[🇷🇺 Русский](CONTRIBUTING.md) · 🇬🇧 English

Thanks for stopping by! RustyCord is built by one person in their spare time, so every extra pair of hands is worth its weight in gold. This document exists so you spend less time guessing and I spend less time asking follow-up questions.

**The short version:**
- There's no money. There's a thank-you, your name in the [CHANGELOG](CHANGELOG.md), and careful review.
- My time is short. I usually reply **every one to two weeks**, sometimes with longer pauses. You're not being ignored; my day job just eats nearly everything.
- Read the [CODE_OF_CONDUCT](CODE_OF_CONDUCT.md): be respectful, no politics, no ethnic slurs.

---

## Pick your role

- 🦀 **Rust developer.** The most needed help: voice and songbird, the Gateway, decoders, bugs the author couldn't solve.
- 🧪 **Tester / QA.** No code required: walk through the app and write bugs down as a list. We badly need a dedicated person for this.
- 🎨 **Svelte / design.** The look and feel is currently one big bug.
- 🐧 **Linux / macOS / Windows 10.** Just run a build and tell us what happened.
- 🔬 **Protocol reverse engineering.** Cross-checking with the official client, documentation.

---

## What we accept and what we don't

**We accept** whatever improves life for users: bug fixes, stability, convenience, new QoL features. The yardstick is simple: *a feature helps a person instead of doing something on their behalf.*

**We do not accept, and close without discussion:**
- selfbots, mass messaging, automation of actions on behalf of an account;
- bypassing Discord's protections, anti-fraud, captchas or limits;
- any sending of the token or user data anywhere except Discord's servers;
- features whose goal is to hide the client from Discord or pass it off as official.

Simple QoL ideas are often better as a **plugin** (see [`plugins/`](plugins)) than baked into the core. If you're not sure where your idea belongs, open a Discussion before writing code.

---

## Building and running

You need [Node.js](https://nodejs.org) ≥ 20, [Rust](https://rustup.rs) ≥ 1.77 and the Tauri system libraries:

- **Ubuntu/Debian:** `sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev libopus-dev cmake`
- **macOS:** Xcode Command Line Tools.
- **Windows:** [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) and [MSVC Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/).

```bash
git clone https://github.com/VinylRUS/Rustycord.git && cd Rustycord
npm install
npm run tauri dev
```

Verbose logs:

```bash
# Linux / macOS
RUST_LOG=debug npm run tauri dev

# Windows (PowerShell)
$env:RUST_LOG="debug"; npm run tauri dev
```

Demo mode without logging in: `npm run dev`, then open `http://localhost:1420`.

---

## Checks before a PR

CI runs the same commands, so run them locally. A PR with red CI will not be reviewed (see [`.github/workflows/ci.yml`](.github/workflows/ci.yml) for the exact flags).

```bash
cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cd ..
npx svelte-check
npm run build
```

---

## Pull request rules

1. **One PR, one problem.** Small changes are easy to review and merge; big rewrites sit for months.
2. **Discuss big things first.** For anything beyond a few dozen lines or anything that changes an event format, open an issue or a discussion first.
3. **Don't break consumers.** If you change an event format or an IPC command, check who reads it on both the Rust and the Svelte side.
4. **Add a test** when you fix a bug that a test can reproduce. A regression caught once should stay caught.
5. **Don't swallow errors for the sake of "stability".** Errors must be visible in the log, but **without tokens, cookies or authorization headers**.
6. **Describe what you checked by hand**: which OS and what exactly you clicked.
7. **No delays, retries or bulk subscriptions "just in case".** Any limit must rest on the protocol and measurements.

### AI-written code

Code written by a neural network is **allowed** when:
- CI is green;
- you understand what's in it and can explain it to a reviewer;
- you've noted in the PR description that you used AI and briefly said what you verified by hand. This isn't a ban, it's a hint about where to look more carefully.

---

## Sensitive areas

Changes here always go through the project owner's review, even from trusted maintainers (see [`.github/CODEOWNERS`](.github/CODEOWNERS)):

- login and token storage;
- the REST client and the Gateway;
- the plugin system;
- Tauri permissions and configuration (`tauri.conf.json`, capabilities);
- CI workflows and releases.

---

## Where to start

> These are candidates gathered from write-ups and logs. Before starting, check in the current code that the problem still exists. The maintainer will turn the relevant items into issues labeled `good first issue` and `help wanted`.

**Rust**
- Voice: make sure songbird has a PCM decoder registered (`symphonia-codec-pcm`) and that microphone audio really reaches the other side. Add a test that checks a live songbird track, not just device capture.
- Gateway: reset the backoff after a successful session; a limiter for repeated IDENTIFY after a streak of 4002s; validation of raw frames from the protocol console; logging the last outgoing frames (without secrets) when a connection closes.
- Member list: a regression test for switching channels inside a guild that has a hidden list.
- Move the token to the OS credential store (Keychain / Credential Manager / libsecret) instead of `localStorage`.

**Svelte / frontend**
- Forward frontend errors to the log with rate limiting (first N with a full stack, then a counter) so spam doesn't drown everything else.
- Look and feel: first compile a list of problems, then fix them.

**QoL ideas**
- Pinned chats on top of the list.
- A single unread list across all servers.
- Custom channel order and grouping.
- Local history search.

**Testers**
- Run a build on Linux, macOS or Windows 10 and describe what works.
- A full UI pass with bug reports.

---

## How to write a bug report

Use the template when creating an issue. It asks for:
- the RustyCord version and your operating system;
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
