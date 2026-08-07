use gpui::AnyWindowHandle;
use gpui_component::{WindowExt as _, notification::Notification};

/// 从同步或异步视图上下文统一推送 gpui-component 窗口通知。
pub fn push(
    window_handle: AnyWindowHandle,
    notification: Notification,
    cx: &mut impl gpui::AppContext,
) {
    cx.update_window(window_handle, move |_, window, cx| {
        window.push_notification(notification, cx);
    })
    .ok();
}
