[中文](./README.md) · [English](./README_EN.md)

# Ruiz

一个基于 [gpui](https://github.com/zed-industries/zed) 的 AI 学习记忆桌面应用：把任意学习材料交给 AI 智能导入，自动整理成知识蓝图并生成复习卡片，再用 [FSRS](https://github.com/open-spaced-repetition/fsrs4anki/wiki) 间隔重复算法安排复习，让记忆事半功倍。

> 开发中项目（WIP）喵～ 功能与界面仍在快速迭代中。

## 功能特性

- **AI 智能导入**：粘贴或导入学习材料后，AI 自动完成「清洗网页噪声 → 归并并命名材料 → 提取知识单元 → 整理知识蓝图 → 生成复习基础题」的完整流水线，整个过程**实时流式显示阶段进度**，可随时取消且不会写入半成品数据。
- **知识蓝图**：导入的材料会被解析为知识单元（含主题、学习目标、重要度、认知动作等），为复习生成提供结构化依据。
- **卡片生成范围**：可按「精简 / AI 建议 / 全面」三种范围生成复习基础题。
- **动态复习**：复习时 AI 会根据知识单元熟练度自适应生成**选择题、填空题、应用题**等动态题型；也可以关闭自适应，统一使用自由输入的简答题。
- **FSRS 间隔重复**：基于 FSRS 调度器安排每张卡片的复习排期（稳定度 / 难度 / 到期时间），科学安排复习节奏。
- **学习小组**：用学习小组组织笔记，支持将笔记移动到不同小组。
- **本地存储**：所有数据保存在本地 SQLite 数据库中，无需联网即可复习。

## 技术栈

| 组件 | 说明 |
| --- | --- |
| [Rust](https://www.rust-lang.org/) | 2024 edition |
| [gpui](https://github.com/zed-industries/zed) | Zed 团队的高性能 UI 框架 |
| [gpui-component](https://github.com/longbridge/gpui-component) | gpui 组件库（按钮、对话框、侧边栏等） |
| [sqlx](https://github.com/launchbadge/sqlx) | SQLite 数据库（异步） |
| [fsrs](https://github.com/open-spaced-repetition/fsrs-rs) | 间隔重复调度算法 |
| [reqwest](https://github.com/seanmonstar/reqwest) | OpenAI 兼容 API 的 SSE 流式请求 |
| [tokio](https://github.com/tokio-rs/tokio) | 异步运行时 |

## 环境要求

- Rust 工具链（支持 2024 edition，建议使用最新 stable：`rustup update stable`）
- 由于依赖 zed 的 `gpui` 等 git 依赖，**首次编译需要较长时间**，请耐心等待
- Linux 下需要安装 gpui 运行所需的系统库（参考 [zed 仓库](https://github.com/zed-industries/zed) 的系统依赖要求）

## 构建与运行

```bash
cargo run --release
```

## 快速开始

1. 启动应用后，在**设置页**填入你的 DeepSeek API Key（默认使用 `deepseek-v4-flash` 模型，也可切换为 `deepseek-v4-pro`）。
2. 在**笔记页**粘贴或导入一篇学习材料。
3. 点击导入，AI 智能导入流水线开始运行：可展开任务面板实时查看「清洗、归并、提取、整理蓝图、生成卡片」等阶段进度。
4. 流水线完成后，资料库中会出现整理好的材料与生成的复习基础题。
5. 在**复习页**按 FSRS 排期进行复习，AI 会根据熟练度动态出题。

## 数据存储

数据库（SQLite）保存在系统数据目录，依次尝试：

1. XDG 数据目录（`ProjectDirs`）
2. `$XDG_DATA_HOME`
3. `$HOME/.local/share`
4. 系统临时目录

全部不可用时回退到当前目录下的 `./ruiz-data`。

## 路线图

- [x] AI 智能导入流水线（SSE 流式进度、阶段展示、可取消）
- [x] 学习小组与笔记管理
- [x] 卡片生成范围（精简 / AI 建议 / 全面）
- [x] 动态复习：自适应 AI 出题（选择 / 填空 / 应用）
- [ ] 任务面板中显示请求编号、模型输出增量与累计耗时
- [ ] 日志与流式视图中的敏感字段统一脱敏

## License

[MIT](./LICENSE)
