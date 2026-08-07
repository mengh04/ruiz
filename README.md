<p align="center">
  <img src="./assets/brand/logo.svg" alt="Ruiz Logo" width="112" />
</p>

<h1 align="center">Ruiz</h1>

<p align="center">面向自主学习的 AI 记忆桌面应用</p>

<p align="center">
  <a href="./README.md">中文</a> · <a href="./README_EN.md">English</a>
</p>

Ruiz 可以将学习材料整理为清晰的知识结构，并结合智能出题与间隔重复，帮助你更高效地理解、记忆和复习内容。

> [!NOTE]
> Ruiz 目前仍处于开发阶段，功能和界面可能随版本更新而调整。

## 主要功能

- **智能导入**：粘贴或导入学习材料，由 AI 自动整理内容。
- **知识蓝图**：从材料中提炼知识单元，形成结构化的学习脉络。
- **动态复习**：根据掌握情况生成不同类型的复习题目。
- **间隔重复**：自动安排复习时间，帮助巩固长期记忆。
- **学习管理**：通过学习小组组织笔记与学习资料。
- **本地保存**：学习数据保存在本地，便于持续管理和复习。

## 使用前准备

- 安装最新稳定版 [Rust](https://www.rust-lang.org/tools/install)。
- 准备用于 AI 功能的 DeepSeek API Key。
- Linux 用户需安装 [Zed](https://github.com/zed-industries/zed/blob/main/docs/src/development/linux.md) 构建所需的系统依赖。

## 构建与运行

克隆项目后，在项目根目录运行：

```bash
cargo run --release
```

首次构建需要下载并编译依赖，可能需要一些时间。

## 快速开始

1. 启动 Ruiz，在设置页配置 DeepSeek API Key。
2. 在笔记页粘贴或导入学习材料。
3. 等待材料整理完成，在资料库中查看生成的知识内容。
4. 前往复习页开始复习，并根据作答情况完成反馈。

## 许可证

本项目基于 [Apache License 2.0](./LICENSE) 开源。
