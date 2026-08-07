use gpui::{AppContext, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root, TitleBar};
use gpui_platform::application;

use crate::assets::Assets;
use crate::settings::{AppSettings, load_config};
use crate::state::AppState;
use crate::ui::views::main_view::MainView;

/// 选择一个可写的数据目录存放数据库。
/// 依次尝试：XDG 数据目录 → XDG_DATA_HOME → HOME/.local/share → 系统临时目录 → 当前目录。
fn data_dir() -> std::path::PathBuf {
    let candidates = [
        directories::ProjectDirs::from("", "", "ruiz").map(|p| p.data_dir().to_path_buf()),
        std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .map(|p| p.join("ruiz")),
        std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .map(|p| p.join(".local/share/ruiz")),
        Some(std::env::temp_dir().join("ruiz-data")),
    ];
    for dir in candidates.into_iter().flatten() {
        if std::fs::create_dir_all(&dir).is_ok() {
            return dir;
        }
    }
    let fallback = std::path::PathBuf::from("./ruiz-data");
    std::fs::create_dir_all(&fallback).expect("无法创建任何可写的数据目录");
    fallback
}

pub fn run() {
    let app = application().with_assets(Assets);
    app.run(|cx| {
        gpui_component::init(cx);
        gpui_tokio::init(cx);
        // 注册配置 global（设置页读写的 Config）
        AppSettings::init(cx);
        if let Err(error) = crate::themes::init(cx) {
            eprintln!("初始化主题失败: {error:#}");
        }

        let data_dir = data_dir();
        if let Err(error) = crate::diagnostics::init(&data_dir) {
            eprintln!("初始化诊断日志失败: {error:#}");
        }
        let db_path = format!("sqlite://{}?mode=rwc", data_dir.join("ruiz.db").display());

        let bounds = WindowBounds::centered(size(px(1280.), px(800.)), cx);

        // 数据库初始化在 tokio runtime 上跑，完成后回到主线程注册全局状态
        let init_task =
            gpui_tokio::Tokio::spawn_result(cx, async move { AppState::new(&db_path).await });
        cx.spawn(async move |cx| {
            match init_task.await {
                Ok(mut state) => {
                    state.configure_ai(&load_config());
                    cx.update(|cx| cx.set_global(state));
                }
                Err(e) => {
                    crate::diagnostics::error(
                        "database.init.failed",
                        "Database initialization failed",
                        serde_json::json!({ "error": format!("{e:#}") }),
                    );
                    eprintln!("初始化数据库失败: {e:?}");
                    return;
                }
            }
            cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("Ruiz".into()),
                        ..TitleBar::title_bar_options()
                    }),
                    window_bounds: Some(bounds),
                    #[cfg(target_os = "linux")]
                    window_background: gpui::WindowBackgroundAppearance::Transparent,
                    #[cfg(target_os = "linux")]
                    window_decorations: Some(gpui::WindowDecorations::Client),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| MainView::new(window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .unwrap();
        })
        .detach();
    });
}
