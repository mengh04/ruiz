<p align="center">
  <img src="./assets/brand/logo.svg" alt="Ruiz Logo" width="112" />
</p>

<h1 align="center">Ruiz</h1>

<p align="center">An AI-powered desktop app for independent learning and long-term memory</p>

<p align="center">
  <a href="./README.md">中文</a> · <a href="./README_EN.md">English</a>
</p>

Ruiz turns learning materials into structured knowledge and combines AI-generated questions with spaced repetition to help you understand, retain, and review information more effectively.

> [!NOTE]
> Ruiz is currently under active development. Features and interfaces may change between versions.

## Features

- **Smart import**: Paste or import learning materials and let AI organize the content.
- **Knowledge blueprints**: Extract knowledge units from source material and connect them into a clear learning structure.
- **Adaptive review**: Generate different types of questions based on your current mastery.
- **Spaced repetition**: Schedule reviews automatically to strengthen long-term memory.
- **Study organization**: Organize notes and learning materials with study groups.
- **Local storage**: Keep your learning data on your device for ongoing study and review.

## Prerequisites

- Install the latest stable version of [Rust](https://www.rust-lang.org/tools/install).
- Prepare a DeepSeek API key for AI features.
- On Linux, install the system dependencies required to build [Zed](https://github.com/zed-industries/zed/blob/main/docs/src/development/linux.md).

## Build and Run

From the project root, run:

```bash
cargo run --release
```

The first build may take some time while dependencies are downloaded and compiled.

## Quick Start

1. Launch Ruiz and configure your DeepSeek API key in Settings.
2. Paste or import learning material from the Notes page.
3. Wait for processing to finish, then review the generated knowledge content in your library.
4. Open the Review page and rate your answers as you study.

## License

This project is licensed under the [Apache License 2.0](./LICENSE).
