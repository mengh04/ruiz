[English](./README_EN.md) · [中文](./README.md)

# Ruiz

An AI-powered learning & memory desktop app built on [gpui](https://github.com/zed-industries/zed): hand any learning material to the AI for smart import — it automatically organizes it into a knowledge blueprint and generates review cards, then schedules reviews with the [FSRS](https://github.com/open-spaced-repetition/fsrs4anki/wiki) spaced-repetition algorithm to make memorization far more effective.

> Work in progress (WIP) — features and UI are still iterating quickly.

## Features

- **AI Smart Import**: paste or import learning material, and the AI runs the full pipeline — "clean web noise → merge & name materials → extract knowledge units → organize knowledge blueprint → generate foundational review questions" — with **real-time streaming stage progress**, cancellable at any time without writing half-baked data.
- **Knowledge Blueprint**: imported materials are parsed into knowledge units (with topics, learning objectives, importance, cognitive actions, etc.), providing a structured basis for review generation.
- **Card Generation Scope**: generate foundational review questions in three scopes — "concise / AI-suggested / comprehensive".
- **Dynamic Review**: during review, the AI adaptively generates dynamic question types — **multiple choice, fill-in-the-blank, applied questions** — based on knowledge-unit mastery; you can also disable adaptivity and use free-form short-answer questions uniformly.
- **FSRS Spaced Repetition**: every card's review date is scheduled by the FSRS scheduler (stability / difficulty / due date) for a scientific review rhythm.
- **Study Groups**: organize notes with study groups and move notes between groups.
- **Local Storage**: all data is stored in a local SQLite database — review works fully offline.

## Tech Stack

| Component | Description |
| --- | --- |
| [Rust](https://www.rust-lang.org/) | 2024 edition |
| [gpui](https://github.com/zed-industries/zed) | High-performance UI framework by the Zed team |
| [gpui-component](https://github.com/longbridge/gpui-component) | gpui component library (buttons, dialogs, sidebar, etc.) |
| [sqlx](https://github.com/launchbadge/sqlx) | SQLite database (async) |
| [fsrs](https://github.com/open-spaced-repetition/fsrs-rs) | Spaced-repetition scheduling algorithm |
| [reqwest](https://github.com/seanmonstar/reqwest) | OpenAI-compatible API SSE streaming requests |
| [tokio](https://github.com/tokio-rs/tokio) | Async runtime |

## Requirements

- Rust toolchain (2024 edition support; latest stable recommended: `rustup update stable`)
- First compilation takes a while due to git dependencies like `gpui` — please be patient
- On Linux, install the system libraries gpui needs at runtime (see the [zed repository](https://github.com/zed-industries/zed) for system dependency requirements)

## Build & Run

```bash
cargo run --release
```

## Quick Start

1. Launch the app and enter your DeepSeek API Key on the **Settings** page (defaults to the `deepseek-v4-flash` model; you can also switch to `deepseek-v4-pro`).
2. Paste or import a learning material on the **Notes** page.
3. Click import and the AI smart-import pipeline starts: expand the task panel to watch stage progress — "clean, merge, extract, organize blueprint, generate cards" — in real time.
4. Once the pipeline finishes, the organized material and generated foundational review questions appear in your library.
5. Review on the **Review** page according to the FSRS schedule — the AI generates questions dynamically based on your mastery.

## Data Storage

The database (SQLite) is stored in the system data directory, trying in order:

1. XDG data directory (`ProjectDirs`)
2. `$XDG_DATA_HOME`
3. `$HOME/.local/share`
4. System temporary directory

Falls back to `./ruiz-data` in the current directory if none are available.

## Roadmap

- [x] AI smart-import pipeline (SSE streaming progress, stage display, cancellable)
- [x] Study groups & note management
- [x] Card generation scope (concise / AI-suggested / comprehensive)
- [x] Dynamic review: adaptive AI question generation (multiple choice / fill-in-the-blank / applied)
- [ ] Show request IDs, model output deltas and cumulative elapsed time in the task panel
- [ ] Unified redaction of sensitive fields in logs and streaming views

## License

[MIT](./LICENSE)
