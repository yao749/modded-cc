//! cc-router-tui 入口: 解析参数 → 找到并连上桌面 app → 进界面 (或 `--check` 只打印状态)。
//! 连接失败的提示在进入备用屏幕**之前**打印到 stderr, 这样用户退出后还看得到。

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use cc_router_tui::app::{App, AppOptions};
use cc_router_tui::client::discovery::{default_data_dir, read_runtime, Platform, RuntimeInfo};
use cc_router_tui::client::dto::{ProxyStatus, Settings, Subscription};
use cc_router_tui::client::{commands, Client, ClientError};
use cc_router_tui::format::Tz;
use cc_router_tui::i18n::{client_error, strings, Lang, Strings};
use cc_router_tui::runtime;
use cc_router_tui::theme::{ColorMode, Theme};
use serde_json::json;

#[derive(Debug, Default, PartialEq, Eq)]
struct Args {
    data_dir: Option<PathBuf>,
    check: bool,
    no_fx: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Parsed {
    Run(Args),
    Help,
    Version,
    /// 参数有误。
    Invalid(BadArg),
}

#[derive(Debug, PartialEq, Eq)]
enum BadArg {
    MissingDataDirPath,
    Unknown(String),
}

/// 参数错误的那句话。与随后追加的 `cli_help` 用同一个 `s`, 两段永远是同一种语言。
fn bad_arg_message(bad: &BadArg, s: &Strings) -> String {
    match bad {
        BadArg::MissingDataDirPath => s.cli_err_missing_data_dir_path.to_string(),
        BadArg::Unknown(arg) => (s.cli_err_unknown_arg)(arg),
    }
}

/// 纯解析, 不碰文件系统: 语言要读 runtime.json, 留给 `main` 在需要打印时再算。
fn parse_args(mut argv: impl Iterator<Item = String>) -> Parsed {
    let mut args = Args::default();
    while let Some(a) = argv.next() {
        match a.as_str() {
            "-h" | "--help" => return Parsed::Help,
            "-V" | "--version" => return Parsed::Version,
            "--check" => args.check = true,
            "--no-fx" => args.no_fx = true,
            "--data-dir" => match argv.next() {
                Some(p) => args.data_dir = Some(PathBuf::from(p)),
                None => return Parsed::Invalid(BadArg::MissingDataDirPath),
            },
            other => return Parsed::Invalid(BadArg::Unknown(other.to_string())),
        }
    }
    Parsed::Run(args)
}

/// 给人看的一句话 + 下一步该做什么。`lang`: 失败那一刻已知的语言——还没拿到设置时是连接前语言
/// (见 [`pre_connect_lang`]), 拿到之后是连接后的界面语言 (见 [`connected_lang`])。
fn explain(err: &ClientError, lang: Lang) -> String {
    let s = strings(lang);
    match err {
        ClientError::Discovery(_) => format!("{}\n{}", client_error(s, err), s.cli_discovery_hint),
        ClientError::NotRunning => format!("{}{}", client_error(s, err), s.cli_not_running_hint),
        ClientError::Disabled => format!("{}{}", client_error(s, err), s.cli_disabled_hint),
        _ => client_error(s, err),
    }
}

/// `ui` 里两类完全不同的失败: 连不上桌面 app (`ClientError`, 走 `explain`) vs 本地终端本身
/// 初始化不了 (比如没有 tty)。故意不把后者塞进 `ClientError::Transport`——那条分支经
/// `client_error` 显示成「网络错误: …」, 会让「请在真正的终端窗口里运行」被误报成网络问题。
enum Failure {
    Client(ClientError),
    Terminal(std::io::Error),
}

impl From<ClientError> for Failure {
    fn from(e: ClientError) -> Self {
        Failure::Client(e)
    }
}

/// 终端初始化失败 (没有可用 tty 等) 时给人看的一句话。`lang`: 见 [`explain`] 同一条注释。
fn terminal_failure_message(err: &std::io::Error, lang: Lang) -> String {
    (strings(lang).cli_terminal_init_failed)(&err.to_string())
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

fn resolve_data_dir(args: &Args) -> Result<PathBuf, ClientError> {
    match &args.data_dir {
        Some(d) => Ok(d.clone()),
        None => Ok(default_data_dir(Platform::current(), env)?),
    }
}

/// 数据目录 + 读一次 runtime.json。正常启动只读这一次: 连接前语言与 [`Client::from_runtime`]
/// 用的是同一份内容。
fn find_runtime(args: &Args) -> Result<(PathBuf, RuntimeInfo), ClientError> {
    let dir = resolve_data_dir(args)?;
    let info = read_runtime(&dir)?;
    Ok((dir, info))
}

/// 连接建立 (甚至尝试连接) 之前能用的语言, 给「未运行 / 未启用 / 数据目录找不到」这类
/// 连接前提示用——这些错误发生在拿到 `get_settings` 之前。偏好取 runtime.json 里桌面端启动时
/// 记下的 `preferred_language` (旧版 app 没有这个字段, 按 `"system"`), 系统标签取同一文件的
/// `system_locale`; `runtime` 为 `None` (数据目录解析不出来 / 文件缺失或损坏) 时只剩环境变量探测。
fn pre_connect_lang(runtime: Option<&RuntimeInfo>, env: impl Fn(&str) -> Option<String>) -> Lang {
    let preferred = runtime.and_then(|r| r.preferred_language.as_deref()).unwrap_or("system");
    Lang::resolve(preferred, runtime.and_then(|r| r.system_locale.as_deref()), env)
}

/// 还没解析参数 (或参数有误) 时的连接前语言: 按默认数据目录尽力读一次 runtime.json。
fn default_pre_connect_lang() -> Lang {
    pre_connect_lang(find_runtime(&Args::default()).ok().map(|(_, rt)| rt).as_ref(), env)
}

/// 连上之后的界面语言: 偏好用 `get_settings` 的实时值, 不用 runtime.json 里启动时记下的那份。
fn connected_lang(settings: &Settings, rt: &RuntimeInfo) -> Lang {
    Lang::resolve(&settings.preferred_language, rt.system_locale.as_deref(), env)
}

/// `lang`: 进来时是连接前语言; 拿到设置后改成连接后的界面语言, 之后的失败 (中途断开等) 由
/// 调用方按它报告。
async fn check(client: Client, lang: &mut Lang) -> Result<(), ClientError> {
    // 设置排第一个请求: 越早拿到实时偏好, 之后的失败就越早能按用户选的语言报告。
    let settings: Settings = client.call(commands::GET_SETTINGS, json!({})).await?;
    *lang = connected_lang(&settings, &client.runtime().await);
    let status: ProxyStatus = client.call(commands::PROXY_STATUS, json!({})).await?;
    let subs: Vec<Subscription> = client.call(commands::LIST_SUBSCRIPTIONS, json!({})).await?;
    let _events = client.events().await?; // 只验证事件流能建立
    let rt = client.runtime().await;
    let s = strings(*lang);

    let dispatchable = subs.iter().filter(|s| s.is_dispatchable).count();
    println!("{}", (s.cli_check_connected)(&rt.app_version, rt.pid));
    println!("{}", (s.cli_check_addr)(&status.base_url));
    println!("{}", (s.cli_check_mode)(&status.mode, status.listen_all));
    println!("{}", (s.cli_check_subs)(subs.len(), dispatchable));
    println!("{}", (s.cli_check_lang)(&settings.preferred_language));
    println!("{}", s.cli_check_events_ok);
    if rt.app_version != env!("CARGO_PKG_VERSION") {
        println!("{}", (s.cli_check_version_mismatch)(env!("CARGO_PKG_VERSION"), &rt.app_version));
    }
    Ok(())
}

/// `lang`: 同 [`check`]——拿到设置后改成连接后的界面语言, 终端初始化失败按它报告。
async fn ui(client: Client, no_fx: bool, lang: &mut Lang) -> Result<(), Failure> {
    // 进界面前先调一次: 既拿到语言设置, 也把「未运行 / 未启用」这类错误挡在备用屏幕之外。
    let settings: Settings = client.call(commands::GET_SETTINGS, json!({})).await?;
    // 取的是建连接时已经读好的 runtime.json 内容, 不是重新读文件。
    *lang = connected_lang(&settings, &client.runtime().await);
    let theme = Theme::new(ColorMode::detect(env));
    let fx_enabled = !no_fx && env("CCR_TUI_NO_FX").is_none_or(|v| v.is_empty() || v == "0") && theme.supports_fx();
    let app = App::new(AppOptions {
        strings: strings(*lang),
        theme,
        fx_enabled,
        now_ms: runtime::unix_ms(),
        tui_version: env!("CARGO_PKG_VERSION"),
        tz: Tz::Local,
    });
    runtime::run(Arc::new(client), app).await.map_err(Failure::Terminal)
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Parsed::Run(a) => a,
        Parsed::Help => {
            // `-h`/`--help` 之前不可能出现过 `--data-dir` (它们赢过之后的一切参数), 按默认
            // 数据目录算连接前语言就够。
            print!("{}", strings(default_pre_connect_lang()).cli_help);
            return ExitCode::SUCCESS;
        }
        Parsed::Version => {
            println!("cc-router-tui {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Parsed::Invalid(bad) => {
            // 参数有误时不知道用户想要哪个 `--data-dir`, 按默认数据目录算连接前语言。
            let s = strings(default_pre_connect_lang());
            eprintln!("{}\n\n{}", bad_arg_message(&bad, s), s.cli_help);
            return ExitCode::from(2);
        }
    };
    let found = find_runtime(&args);
    let mut lang = pre_connect_lang(found.as_ref().ok().map(|(_, rt)| rt), env);
    let result: Result<(), Failure> = match found.and_then(|(dir, rt)| Client::from_runtime(dir, rt)) {
        Ok(client) if args.check => check(client, &mut lang).await.map_err(Failure::Client),
        Ok(client) => ui(client, args.no_fx, &mut lang).await,
        Err(e) => Err(Failure::Client(e)),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Client(e)) => {
            eprintln!("{}", explain(&e, lang));
            ExitCode::FAILURE
        }
        Err(Failure::Terminal(e)) => {
            eprintln!("{}", terminal_failure_message(&e, lang));
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Parsed {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_arguments_means_run_the_ui() {
        assert_eq!(parse(&[]), Parsed::Run(Args::default()));
    }

    #[test]
    fn flags_combine_in_any_order() {
        assert_eq!(
            parse(&["--no-fx", "--data-dir", "/tmp/x", "--check"]),
            Parsed::Run(Args { data_dir: Some("/tmp/x".into()), check: true, no_fx: true })
        );
    }

    #[test]
    fn help_and_version_win_over_everything_after_them() {
        assert_eq!(parse(&["-h", "--bogus"]), Parsed::Help);
        assert_eq!(parse(&["--version"]), Parsed::Version);
    }

    #[test]
    fn bad_input_is_reported_not_ignored() {
        assert_eq!(parse(&["--data-dir"]), Parsed::Invalid(BadArg::MissingDataDirPath));
        assert_eq!(parse(&["--wat"]), Parsed::Invalid(BadArg::Unknown("--wat".into())));
    }

    #[test]
    fn bad_input_is_worded_by_strings() {
        for lang in Lang::ALL {
            let s = strings(lang);
            assert_eq!(bad_arg_message(&BadArg::MissingDataDirPath, s), s.cli_err_missing_data_dir_path, "{lang:?}");
            assert_eq!(bad_arg_message(&BadArg::Unknown("--wat".into()), s), (s.cli_err_unknown_arg)("--wat"), "{lang:?}");
        }
    }

    fn runtime_info(json: &str) -> RuntimeInfo {
        serde_json::from_str(json).unwrap()
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    /// 桌面端显式选了与系统不同的语言时, 连接前提示 (「未启用」等) 跟随桌面端的选择, 不跟随系统。
    #[test]
    fn pre_connect_lang_follows_the_preference_recorded_in_runtime_json() {
        let rt = runtime_info(
            r#"{"pid":1,"app_version":"x","http_port":1,"https_port":null,"ca_pem_path":null,"local_secret":"s",
                "system_locale":"zh-Hans-CN","preferred_language":"ja"}"#,
        );
        assert_eq!(pre_connect_lang(Some(&rt), no_env), Lang::Ja);
    }

    /// 旧版桌面端写的 runtime.json 没有 `preferred_language`: 按「跟随系统」, 用同一文件的系统标签。
    #[test]
    fn pre_connect_lang_without_preference_falls_back_to_the_system_locale() {
        let rt = runtime_info(
            r#"{"pid":1,"app_version":"x","http_port":1,"https_port":null,"ca_pem_path":null,"local_secret":"s",
                "system_locale":"ja-JP"}"#,
        );
        assert_eq!(pre_connect_lang(Some(&rt), |k| (k == "LANG").then(|| "zh_CN.UTF-8".to_string())), Lang::Ja);
        assert_eq!(pre_connect_lang(None, |k| (k == "LANG").then(|| "zh_CN.UTF-8".to_string())), Lang::Zh);
    }

    /// 入口真正走的那条路: 从数据目录读出 runtime.json, 连接前语言与建连接用的是同一份内容。
    #[test]
    fn find_runtime_reads_the_file_that_decides_the_pre_connect_language() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("runtime.json"),
            r#"{"pid":1,"app_version":"x","http_port":1,"https_port":null,"ca_pem_path":null,"local_secret":"s",
                "system_locale":"en-US","preferred_language":"zh"}"#,
        )
        .unwrap();
        let args = Args { data_dir: Some(dir.path().to_path_buf()), ..Args::default() };
        let (found_dir, rt) = find_runtime(&args).unwrap();
        assert_eq!(found_dir, dir.path());
        assert_eq!(pre_connect_lang(Some(&rt), no_env), Lang::Zh);
    }

    /// 连上之后以 `get_settings` 的实时值为准, runtime.json 里启动时记下的偏好不再参与。
    #[test]
    fn connected_lang_uses_the_live_preference_not_the_recorded_one() {
        let rt = runtime_info(
            r#"{"pid":1,"app_version":"x","http_port":1,"https_port":null,"ca_pem_path":null,"local_secret":"s",
                "system_locale":"zh-Hans-CN","preferred_language":"ja"}"#,
        );
        let settings = Settings { preferred_language: "en".into(), tui_enabled: true, auth_enabled: true };
        assert_eq!(connected_lang(&settings, &rt), Lang::En);
        let follow = Settings { preferred_language: "system".into(), ..settings };
        assert_eq!(connected_lang(&follow, &rt), Lang::Zh);
    }

    /// H1: 终端初始化失败 (没有 tty 等) 不该被当成「网络错误」报出来——那是 `ClientError::Transport`
    /// 的文案, 和「连不上桌面 app」是两码事, 会把用户带偏。
    #[test]
    fn terminal_failure_message_is_not_reported_as_a_network_error() {
        let err = std::io::Error::other("x");
        for lang in Lang::ALL {
            let s = strings(lang);
            let msg = terminal_failure_message(&err, lang);
            assert_eq!(msg, (s.cli_terminal_init_failed)("x"), "{lang:?}");
            let network_prefix = (s.err_network)("");
            assert!(!msg.contains(network_prefix.trim_end()), "{lang:?}: {msg}");
        }
    }
}
