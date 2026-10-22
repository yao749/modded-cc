//! 托盘菜单的构造与就地刷新。
//!
//! 菜单建好后把会变的几项 (状态行 / 订阅行 / 复制 / 更新 / 开机自启) 的句柄存进
//! `TrayState`, 之后状态变化只 `set_text` / `set_checked`, 不重建菜单 —— macOS 上
//! `set_menu` 会把用户正开着的菜单直接关掉。只有切换语言才整个重建 (`rebuild_menu`)。

use std::sync::Mutex;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::{AppHandle, Manager, Wry};
use tracing::warn;

use super::strings::{TrayLocale, TrayStrings};
use super::TRAY_ID;

/// 菜单项 id。`on_menu_event` 按 id 分发, 重建菜单时 id 保持不变, 启动时挂的
/// 那一份 handler 对新菜单继续有效 (见 `super::rebuild_menu` 的注释)。
pub mod ids {
    pub const SHOW: &str = "show";
    pub const QUIT: &str = "quit";
    pub const OPEN_SUBS: &str = "open_subs";
    pub const OPEN_LIVE: &str = "open_live";
    pub const OPEN_LOGS: &str = "open_logs";
    pub const COPY_ENV: &str = "copy_env";
    pub const CHECK_UPDATE: &str = "check_update";
    pub const AUTOSTART: &str = "autostart";
}

/// 菜单上显示的运行时状态, 由 `super::refresh` 从 `AppState` 汇总。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraySnapshot {
    /// 代理实际监听地址 (不带 scheme); None = 两路 listener 都没绑上
    pub address: Option<String>,
    /// 全部订阅数 (含停用)
    pub total: usize,
    /// 启用中的订阅数
    pub enabled: usize,
    /// 启用中但调度器此刻不会选它的 (故障 / 冷却 / 限额满), 与 `SubscriptionDto.is_dispatchable` 同一判定
    pub down: usize,
}

/// 订阅行文案。停用是用户的主动选择, 不算「不可用」, 所以分母是启用中的订阅。
pub fn subs_line(s: &TrayStrings, snap: &TraySnapshot) -> String {
    if snap.total == 0 {
        s.subs_none.to_string()
    } else if snap.enabled == 0 {
        s.subs_all_disabled.to_string()
    } else if snap.down == 0 {
        (s.subs_ok)(snap.enabled)
    } else {
        (s.subs_some_down)(snap.enabled, snap.down)
    }
}

/// 会随状态变化的菜单项句柄。
struct TrayItems {
    status: MenuItem<Wry>,
    subs: MenuItem<Wry>,
    copy_env: MenuItem<Wry>,
    update: MenuItem<Wry>,
    autostart: CheckMenuItem<Wry>,
}

struct Inner {
    locale: TrayLocale,
    items: Option<TrayItems>,
    /// None = 还没汇总过一次 (启动后极短的窗口), 状态行显示「启动中」
    snapshot: Option<TraySnapshot>,
    autostart: bool,
    /// 前端检测到的新版本号 (`set_tray_update` 推过来的)
    update: Option<String>,
    /// 刚复制完环境变量, 「复制」那一项临时显示「已复制」
    copied: bool,
}

/// 托盘的全部可变状态, `app.manage` 一份。
pub struct TrayState {
    inner: Mutex<Inner>,
}

impl TrayState {
    pub fn new(locale: TrayLocale) -> Self {
        Self {
            inner: Mutex::new(Inner {
                locale,
                items: None,
                snapshot: None,
                autostart: false,
                update: None,
                copied: false,
            }),
        }
    }

    pub fn set_snapshot(&self, snapshot: TraySnapshot, autostart: bool) {
        if let Ok(mut g) = self.inner.lock() {
            g.snapshot = Some(snapshot);
            g.autostart = autostart;
        }
    }

    pub fn set_update(&self, version: Option<String>) {
        if let Ok(mut g) = self.inner.lock() {
            g.update = version;
        }
    }

    pub fn set_copied(&self, copied: bool) {
        if let Ok(mut g) = self.inner.lock() {
            g.copied = copied;
        }
    }

    /// 按语言整个重建菜单并挂到托盘上。只 `set_menu`, 不碰 `on_menu_event`。
    /// 调用方负责在主线程上调用 (muda 的 NSMenu 只能主线程碰)。
    pub fn install_menu(&self, app: &AppHandle, locale: TrayLocale) -> tauri::Result<()> {
        let Some(tray) = app.tray_by_id(TRAY_ID) else {
            warn!("tray icon '{TRAY_ID}' 不存在, 跳过菜单安装");
            return Ok(());
        };
        let (menu, items) = build_menu(app, locale.strings())?;
        tray.set_menu(Some(menu))?;
        if let Ok(mut g) = self.inner.lock() {
            g.locale = locale;
            g.items = Some(items);
        }
        self.apply(app);
        Ok(())
    }

    /// 把当前状态写到菜单项文字与托盘提示上。主线程调用。
    pub fn apply(&self, app: &AppHandle) {
        let Ok(g) = self.inner.lock() else { return };
        let Some(items) = g.items.as_ref() else { return };
        let s = g.locale.strings();

        let status = match &g.snapshot {
            None => s.status_starting.to_string(),
            Some(snap) => match &snap.address {
                Some(addr) => (s.status_running)(addr),
                None => s.status_stopped.to_string(),
            },
        };
        let subs = match &g.snapshot {
            None => String::new(),
            Some(snap) => subs_line(s, snap),
        };
        let copy = if g.copied { s.copied } else { s.copy_env };
        let update = match &g.update {
            Some(v) => (s.update_available)(v),
            None => s.check_update.to_string(),
        };

        let results = [
            items.status.set_text(status),
            items.subs.set_text(subs),
            items.subs.set_enabled(g.snapshot.is_some()),
            items.copy_env.set_text(copy),
            items.update.set_text(update),
            items.autostart.set_checked(g.autostart),
        ];
        for r in results {
            if let Err(e) = r {
                warn!(error = %e, "tray menu item update failed");
            }
        }

        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            let (running, down) = g
                .snapshot
                .as_ref()
                .map(|snap| (snap.address.is_some(), snap.down))
                .unwrap_or((true, 0));
            let tip = (s.tooltip)(running, down, g.update.is_some());
            if let Err(e) = tray.set_tooltip(Some(tip)) {
                warn!(error = %e, "tray set_tooltip failed");
            }
        }
    }
}

fn build_menu(app: &AppHandle, s: &TrayStrings) -> tauri::Result<(Menu<Wry>, TrayItems)> {
    // 版本号不翻译; 纯展示, 禁用即灰字
    let version = MenuItem::new(
        app,
        format!("cc-router v{}", app.package_info().version),
        false,
        None::<&str>,
    )?;
    let status = MenuItem::new(app, s.status_starting, false, None::<&str>)?;
    // 订阅行可点 (→ 订阅管理页): 灰字在部分 Linux 桌面上几乎看不清, 而订阅出问题时用户最想点进去
    let subs = MenuItem::with_id(app, ids::OPEN_SUBS, "", false, None::<&str>)?;
    let show = MenuItem::with_id(app, ids::SHOW, s.show_window, true, None::<&str>)?;
    let open_live = MenuItem::with_id(app, ids::OPEN_LIVE, s.open_live, true, None::<&str>)?;
    let open_logs = MenuItem::with_id(app, ids::OPEN_LOGS, s.open_logs, true, None::<&str>)?;
    let copy_env = MenuItem::with_id(app, ids::COPY_ENV, s.copy_env, true, None::<&str>)?;
    let update = MenuItem::with_id(app, ids::CHECK_UPDATE, s.check_update, true, None::<&str>)?;
    let autostart = CheckMenuItem::with_id(app, ids::AUTOSTART, s.autostart, true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, ids::QUIT, s.quit, true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &version,
            &status,
            &subs,
            &PredefinedMenuItem::separator(app)?,
            &show,
            &open_live,
            &open_logs,
            &PredefinedMenuItem::separator(app)?,
            &copy_env,
            &PredefinedMenuItem::separator(app)?,
            &update,
            &autostart,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    Ok((
        menu,
        TrayItems {
            status,
            subs,
            copy_env,
            update,
            autostart,
        },
    ))
}

/// `app.state::<TrayState>()` 的可失败版本: 代理可能在托盘装好之前就绑定完端口并请求刷新。
pub fn tray_state(app: &AppHandle) -> Option<tauri::State<'_, TrayState>> {
    app.try_state::<TrayState>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray::strings::ZH;

    fn snap(total: usize, enabled: usize, down: usize) -> TraySnapshot {
        TraySnapshot {
            address: Some("127.0.0.1:23456".into()),
            total,
            enabled,
            down,
        }
    }

    #[test]
    fn subs_line_covers_every_case() {
        assert_eq!(subs_line(&ZH, &snap(0, 0, 0)), "还没有订阅");
        assert_eq!(subs_line(&ZH, &snap(3, 0, 0)), "订阅全部已停用");
        assert_eq!(subs_line(&ZH, &snap(6, 6, 0)), "6 个订阅 · 全部可用");
        // 停用的不进分母, 也不算「不可用」
        assert_eq!(subs_line(&ZH, &snap(6, 5, 1)), "5 个订阅 · 1 个不可用");
    }
}
