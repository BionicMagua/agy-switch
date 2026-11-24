#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod winsec;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::os::windows::process::CommandExt;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::Manager;

const TARGET: &str = "gemini:antigravity";
const USER: &str = "antigravity";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;

fn es<E: ToString>(e: E) -> String {
    e.to_string()
}

fn env_path(k: &str) -> Result<PathBuf, String> {
    std::env::var_os(k)
        .map(PathBuf::from)
        .ok_or_else(|| format!("环境变量 {k} 未设置"))
}

fn gemini_dir() -> Result<PathBuf, String> {
    Ok(env_path("USERPROFILE")?.join(".gemini"))
}
fn fallback_file() -> Result<PathBuf, String> {
    Ok(gemini_dir()?.join("jetski-standalone-oauth-token"))
}
fn keyring_marker() -> Result<PathBuf, String> {
    Ok(gemini_dir()?
        .join("cache")
        .join("antigravity-keyring-unavailable"))
}
fn adc_file() -> Result<PathBuf, String> {
    Ok(env_path("APPDATA")?
        .join("gcloud")
        .join("application_default_credentials.json"))
}
/// 系统程序一律用绝对路径调用，避免从 exe 所在目录（如"下载"文件夹）加载同名程序。
fn system_exe(rel: &str) -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join(rel)
}
fn powershell() -> Command {
    let mut c = Command::new(system_exe(
        r"System32\WindowsPowerShell\v1.0\powershell.exe",
    ));
    c.args(["-NoProfile", "-NonInteractive", "-Command"])
        .creation_flags(CREATE_NO_WINDOW);
    c
}

/// 卸载信息里的 DisplayIcon 形如 `"C:\...\Antigravity.exe",0`，取出 exe 路径。
fn icon_exe(icon: &str) -> Option<PathBuf> {
    let s = icon.trim();
    let s = match s.rsplit_once(',') {
        Some((head, idx)) if idx.trim().parse::<i32>().is_ok() => head,
        _ => s,
    };
    let s = s.trim().trim_matches('"').trim();
    (!s.is_empty()).then(|| PathBuf::from(s))
}

/// Antigravity 安装目录：默认 %LOCALAPPDATA%\Programs\antigravity；
/// 装在别处时从卸载注册表（发布者 Google、名称 "Antigravity x.y.z"）找。
fn app_root() -> Result<PathBuf, String> {
    let default = env_path("LOCALAPPDATA")?
        .join("Programs")
        .join("antigravity");
    if default.join("Antigravity.exe").is_file() {
        return Ok(default);
    }
    static FOUND: OnceLock<PathBuf> = OnceLock::new();
    if let Some(p) = FOUND.get() {
        return Ok(p.clone());
    }
    let out = powershell()
        .arg(r"[Console]::OutputEncoding = New-Object Text.UTF8Encoding $false; $k = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*','HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*','HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*'; Get-ItemProperty $k -ErrorAction SilentlyContinue | Where-Object { $_.Publisher -eq 'Google' -and $_.DisplayName -match '^Antigravity( [0-9]|$)' } | ForEach-Object { $_.DisplayIcon }")
        .output();
    if let Ok(out) = out {
        for line in String::from_utf8_lossy(&out.stdout)
            .trim_start_matches('\u{feff}')
            .lines()
        {
            if let Some(exe) = icon_exe(line) {
                let named = exe
                    .file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case("Antigravity.exe"));
                if named && exe.is_file() {
                    if let Some(dir) = exe.parent() {
                        return Ok(FOUND.get_or_init(|| dir.to_path_buf()).clone());
                    }
                }
            }
        }
    }
    Ok(default)
}
fn app_exe() -> Result<PathBuf, String> {
    Ok(app_root()?.join("Antigravity.exe"))
}
fn require_app() -> Result<PathBuf, String> {
    let exe = app_exe()?;
    if exe.is_file() {
        Ok(exe)
    } else {
        Err(format!(
            "未找到 Antigravity（{}）。请确认已安装 Antigravity 桌面版",
            exe.display()
        ))
    }
}
fn profiles_dir() -> Result<PathBuf, String> {
    let d = env_path("LOCALAPPDATA")?
        .join("agy-switch")
        .join("profiles");
    fs::create_dir_all(&d).map_err(es)?;
    Ok(d)
}
fn active_file() -> Result<PathBuf, String> {
    Ok(profiles_dir()?.join("..").join("active.txt"))
}

// ---------- StoredToken 解析 ----------

#[derive(Serialize, Deserialize, Clone, Default)]
struct Summary {
    email: String,
    auth_method: String,
    project_id: String,
    region: String,
    user_tier: String,
    tier_display_name: String,
    expiry: String,
}

fn summarize(v: &Value) -> Summary {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    Summary {
        email: s("id_token_email"),
        auth_method: s("auth_method"),
        project_id: s("project_id"),
        region: s("region"),
        user_tier: s("user_tier"),
        tier_display_name: s("tier_display_name"),
        expiry: v["token"]["expiry"].as_str().unwrap_or("").to_string(),
    }
}

/// 退出登录后后端可能留下不含 token 的残余条目，需要排除。
fn has_valid_token(v: &Value) -> bool {
    let t = &v["token"];
    ["refresh_token", "access_token"]
        .iter()
        .any(|k| t[*k].as_str().map_or(false, |s| !s.is_empty()))
}

// 只在身份和登录模式都一致时更新档案。active.txt 只是上次选择，不能证明当前身份。
fn same_login(a: &Value, b: &Value) -> bool {
    if !has_valid_token(a) || !has_valid_token(b) {
        return false;
    }
    let sa = summarize(a);
    let sb = summarize(b);
    let method = |s: &str| {
        if s.is_empty() {
            "consumer".to_string()
        } else {
            s.to_string()
        }
    };
    if method(&sa.auth_method) != method(&sb.auth_method)
        || sa.project_id != sb.project_id
        || sa.region != sb.region
        || a.get("wif_provider") != b.get("wif_provider")
    {
        return false;
    }
    if !sa.email.is_empty() && !sb.email.is_empty() {
        return sa.email.eq_ignore_ascii_case(&sb.email);
    }
    let ar = a["token"]["refresh_token"].as_str().unwrap_or("");
    let br = b["token"]["refresh_token"].as_str().unwrap_or("");
    if ar.is_empty() && br.is_empty() {
        // 没有邮箱也没有 refresh token 时，Antigravity 自己也只按 access token 区分
        let aa = a["token"]["access_token"].as_str().unwrap_or("");
        let ba = b["token"]["access_token"].as_str().unwrap_or("");
        return !aa.is_empty() && aa == ba;
    }
    !ar.is_empty() && ar == br
}

/// 能否认出这个登录是谁：有邮箱或 refresh token。认不出时无法判断它有没有保存过。
fn identifiable(v: &Value) -> bool {
    !summarize(v).email.is_empty()
        || !v["token"]["refresh_token"]
            .as_str()
            .unwrap_or("")
            .is_empty()
}

fn parse_stored(raw: &str, source: &str) -> Result<Value, String> {
    // 不把 JSON 原文或解析器上下文带入错误，避免泄露凭据。
    serde_json::from_str(raw.trim_start_matches('\u{feff}'))
        .map_err(|_| format!("{source} 中的登录凭据不是有效 JSON"))
}

/// 降级文件中的有效登录；文件不存在或没有 token 时为 None。
fn read_fallback() -> Result<Option<String>, String> {
    match fs::read_to_string(fallback_file()?) {
        Ok(raw) => {
            let raw = raw.trim_start_matches('\u{feff}').to_string();
            let v = parse_stored(&raw, "降级文件")?;
            Ok(has_valid_token(&v).then_some(raw))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("读取降级凭据失败: {e}")),
    }
}

/// 凭据管理器中的登录：(有效登录或 None=退出状态, 写入时间)。
fn read_keyring() -> Result<Option<(Option<String>, SystemTime)>, String> {
    match winsec::cred_read_at(TARGET)? {
        Some((b, at)) => {
            let raw = String::from_utf8(b).map_err(|_| "Windows 凭据不是 UTF-8".to_string())?;
            let v = parse_stored(&raw, TARGET)?;
            Ok(Some((has_valid_token(&v).then_some(raw), at)))
        }
        None => Ok(None),
    }
}

/// 读取当前登录凭据：优先凭据管理器，其次降级文件。
/// 有 Keyring 超时标记时 Antigravity 可能正在用降级文件：两边都有有效登录就取后写入的那个；
/// 标记可能是过期残留，所以不盲目信文件，文件坏了也不影响读凭据管理器。
fn read_live() -> Result<Option<String>, String> {
    let keyring = read_keyring();
    if keyring_marker()?.exists() {
        if let Ok(Some(file_raw)) = read_fallback() {
            let file_at = fs::metadata(fallback_file()?)
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
            // 凭据管理器后写入就以它为准，包括后来的退出状态（无 token）
            if let Ok(Some((key_raw, key_at))) = &keyring {
                if *key_at > file_at {
                    return Ok(key_raw.clone());
                }
            }
            return Ok(Some(file_raw));
        }
    }
    let read_error = match keyring {
        // 无 token 的条目是退出状态，不用旧的降级文件把它重新认作已登录。
        Ok(Some((raw, _))) => return Ok(raw),
        Ok(None) => None,
        Err(e) => Some(e),
    };
    match (read_fallback()?, read_error) {
        (Some(raw), _) => Ok(Some(raw)),
        (None, Some(e)) if !fallback_file()?.exists() => Err(e),
        (None, _) => Ok(None),
    }
}

// ---------- 档案存储（DPAPI 加密） ----------

#[derive(Serialize, Deserialize)]
struct Profile {
    name: String,
    saved_at: u64,
    token_json: String,
    adc: Option<String>,
}

#[derive(Serialize)]
struct ProfileInfo {
    name: String,
    saved_at: u64,
    summary: Summary,
    has_adc: bool,
    active: bool,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn valid_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 50 {
        return Err("名称需为 1~50 个字符".into());
    }
    if n != name
        || name
            .chars()
            .any(|c| c.is_control() || "\\/:*?\"<>|.".contains(c))
        || name.ends_with(' ')
    {
        return Err("名称不能包含 \\ / : * ? \" < > | . 等字符，且首尾不能有空格".into());
    }
    Ok(())
}

fn profile_path(name: &str) -> Result<PathBuf, String> {
    valid_name(name)?;
    Ok(profiles_dir()?.join(format!("{name}.bin")))
}

fn load_profile(name: &str) -> Result<Profile, String> {
    let enc = fs::read(profile_path(name)?).map_err(|e| format!("读取档案失败: {e}"))?;
    let plain = winsec::unprotect(&enc)?;
    serde_json::from_slice(&plain).map_err(|e| format!("档案损坏: {e}"))
}

fn store_profile(p: &Profile) -> Result<(), String> {
    let plain = serde_json::to_vec(p).map_err(es)?;
    let enc = winsec::protect(&plain)?;
    // 先写临时文件再替换，写到一半中断也不会损坏原档案
    let path = profile_path(&p.name)?;
    let tmp = path.with_extension("bin.tmp");
    fs::write(&tmp, enc).map_err(es)?;
    fs::rename(&tmp, &path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("保存档案失败: {e}")
    })
}

fn get_active() -> Option<String> {
    fs::read_to_string(active_file().ok()?)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn set_active(name: &str) -> Result<(), String> {
    fs::write(active_file()?, name).map_err(es)
}

// ---------- 进程管理 ----------

struct AppProcs {
    pids: Vec<u32>,
    /// 本会话里有读不到路径的 Antigravity 进程：多半以管理员身份运行，本工具无权关闭。
    locked: bool,
}
impl AppProcs {
    fn running(&self) -> bool {
        self.locked || !self.pids.is_empty()
    }
}
const LOCKED_MSG: &str = "检测到以管理员身份运行的 Antigravity，本工具无权关闭它。请先手动完全退出 Antigravity（包括托盘图标），再重试";

fn app_procs() -> Result<AppProcs, String> {
    app_procs_at(&app_root()?)
}

fn app_procs_at(root: &std::path::Path) -> Result<AppProcs, String> {
    // 只识别此安装目录的进程，避免关闭名称相同的 IDE 或其他语言服务器。
    // 按名称 Get-Process 在进程不存在时会让 powershell.exe 返回 1；
    // 切换刚关闭应用时这是正常状态，所以先枚举，再筛选名称。
    let output = powershell()
        .arg(r"$ErrorActionPreference = 'Stop'; $exe = Join-Path $env:AGY_ROOT 'Antigravity.exe'; $ls = Join-Path $env:AGY_ROOT 'resources\bin\language_server.exe'; $sid = (Get-Process -Id $PID).SessionId; Get-Process | Where-Object { $_.ProcessName -in @('Antigravity', 'language_server') } | ForEach-Object { if ($_.Path -eq $exe -or $_.Path -eq $ls) { $_.Id } elseif (-not $_.Path -and $_.ProcessName -eq 'Antigravity' -and $_.SessionId -eq $sid) { 'locked' } }")
        .env("AGY_ROOT", root)
        .output()
        .map_err(es)?;
    if !output.status.success() {
        return Err("无法检查 Antigravity 进程状态".into());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(AppProcs {
        pids: text
            .lines()
            .filter_map(|s| s.trim().parse::<u32>().ok())
            .collect(),
        locked: text.lines().any(|s| s.trim() == "locked"),
    })
}

fn kill_app() -> Result<(), String> {
    let procs = app_procs()?;
    if procs.locked {
        return Err(LOCKED_MSG.into());
    }
    for pid in procs.pids {
        Command::new(system_exe(r"System32\taskkill.exe"))
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(es)?;
    }
    for _ in 0..20 {
        if !app_procs()?.running() {
            break;
        }
        thread::sleep(Duration::from_millis(300));
    }
    // 给文件句柄 / 凭据写入一点收尾时间
    thread::sleep(Duration::from_millis(500));
    let left = app_procs()?;
    if left.locked {
        return Err(LOCKED_MSG.into());
    }
    if left.running() {
        return Err("Antigravity 仍在运行，未写入凭据。请先关闭应用后重试".into());
    }
    Ok(())
}

fn launch_app() -> Result<(), String> {
    let exe = require_app()?;
    // 本工具以管理员身份运行时，经资源管理器启动，让 Antigravity 保持普通权限。
    let mut cmd = if winsec::is_elevated().unwrap_or(false) {
        let mut c = Command::new(system_exe("explorer.exe"));
        c.raw_arg(format!("\"{}\"", exe.display()));
        c
    } else {
        let mut c = Command::new(&exe);
        c.creation_flags(DETACHED_PROCESS);
        c
    };
    cmd.current_dir(exe.parent().ok_or("Antigravity 安装路径无效")?)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn()
        .map_err(|e| format!("无法启动 Antigravity: {e}"))?;
    // 创建启动进程不等于应用已经运行，避免失败时仍提示“已切换”。
    for _ in 0..30 {
        if app_procs()?.running() {
            thread::sleep(Duration::from_millis(300));
            if app_procs()?.running() {
                return Ok(());
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
    Err("已发出启动请求，但未检测到 Antigravity 持续运行。账号档案仍保留，请手动打开 Antigravity 后重试".into())
}

/// 应用已关闭后，后续写入失败也要尝试重新打开，保留原始错误。
fn finish_restart(
    operation: Result<(), String>,
    restart: bool,
    launch: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if !restart {
        return operation;
    }
    match (operation, launch()) {
        (Ok(()), launched) => launched,
        (Err(error), Ok(())) => Err(format!("{error}。已重新打开 Antigravity")),
        (Err(error), Err(start_error)) => Err(format!("{error}。重新打开也失败: {start_error}")),
    }
}

/// 当前登录还没存成档案时拒绝会覆盖它的操作，否则这个登录就找不回来了。
fn require_saved(state: &State, action: &str) -> Result<(), String> {
    if let Some(error) = &state.current_error {
        return Err(format!("无法确认当前登录，未执行{action}: {error}"));
    }
    if let (Some(c), None, true) = (&state.current, &state.active, state.identifiable) {
        let who = if c.email.is_empty() {
            "当前账号"
        } else {
            &c.email
        };
        return Err(format!(
            "{who} 还没有保存为档案，{action}后将无法切回。请先点「保存当前」保存，再{action}"
        ));
    }
    Ok(())
}

// ---------- Tauri 命令 ----------

/// 命令在后台线程执行：改动凭据或档案的操作一次只跑一个。
static OP: Mutex<()> = Mutex::new(());
fn op_lock() -> std::sync::MutexGuard<'static, ()> {
    OP.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Serialize, Default)]
struct State {
    current: Option<Summary>,
    profiles: Vec<ProfileInfo>,
    active: Option<String>,
    app_running: bool,
    process_error: Option<String>,
    current_error: Option<String>,
    profile_errors: Vec<String>,
    #[serde(skip)]
    identifiable: bool,
}

#[tauri::command(async)]
fn get_state() -> Result<State, String> {
    let saved = fs::read_dir(profiles_dir()?)
        .map_err(es)?
        .map(|entry| {
            let path = entry.map_err(es)?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("bin") {
                return Ok(None);
            }
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("档案文件名无效")?;
            Ok(Some(
                load_profile(name).map_err(|e| format!("档案「{name}」读取失败: {e}")),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?
        .into_iter()
        .flatten()
        .collect();
    Ok(build_state(read_live(), saved, get_active(), app_procs()))
}

fn build_state(
    live: Result<Option<String>, String>,
    saved: Vec<Result<Profile, String>>,
    last_active: Option<String>,
    processes: Result<AppProcs, String>,
) -> State {
    let (live, current_error) =
        match live.and_then(|raw| raw.map(|s| parse_stored(&s, "当前登录")).transpose()) {
            Ok(live) => (live, None),
            Err(error) => (None, Some(error)),
        };
    let current = live.as_ref().map(summarize);
    let mut profiles = Vec::new();
    let mut profile_errors = Vec::new();
    for profile in saved {
        let parsed = profile.and_then(|p| {
            let stored = parse_stored(&p.token_json, "档案")
                .map_err(|e| format!("档案「{}」读取失败: {e}", p.name))?;
            Ok((p, stored))
        });
        if let Ok((p, stored)) = parsed {
            let summary = summarize(&stored);
            profiles.push(ProfileInfo {
                active: live.as_ref().is_some_and(|v| same_login(v, &stored)),
                name: p.name,
                saved_at: p.saved_at,
                summary,
                has_adc: p.adc.is_some(),
            });
        } else if let Err(error) = parsed {
            profile_errors.push(error);
        }
    }
    profiles.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    let active = profiles
        .iter()
        .find(|p| p.active && Some(&p.name) == last_active.as_ref())
        .or_else(|| profiles.iter().find(|p| p.active))
        .map(|p| p.name.clone());
    for p in &mut profiles {
        p.active = active.as_ref() == Some(&p.name);
    }
    let (app_running, process_error) = match processes {
        Ok(processes) => (processes.running(), None),
        Err(error) => (false, Some(error)),
    };
    State {
        current,
        profiles,
        active,
        app_running,
        process_error,
        current_error,
        profile_errors,
        identifiable: live.as_ref().is_some_and(identifiable),
    }
}

#[tauri::command(async)]
fn save_profile(name: String, include_adc: bool) -> Result<(), String> {
    let _op = op_lock();
    valid_name(&name)?;
    let token_json = read_live()?.ok_or("当前没有有效登录：请先在 Antigravity 中登录，再保存")?;
    if profile_path(&name)?.exists() {
        let old = load_profile(&name)?;
        if !same_login(
            &parse_stored(&old.token_json, "档案")?,
            &parse_stored(&token_json, "当前登录")?,
        ) {
            return Err("此名称已用于其他账号或项目，请给新账号使用不同的档案名".into());
        }
    }
    let adc = if include_adc {
        fs::read_to_string(adc_file()?).ok()
    } else {
        None
    };
    store_profile(&Profile {
        name: name.clone(),
        saved_at: now(),
        token_json,
        adc,
    })?;
    set_active(&name)
}

#[tauri::command(async)]
fn delete_profile(name: String) -> Result<(), String> {
    let _op = op_lock();
    fs::remove_file(profile_path(&name)?).map_err(es)?;
    if get_active().as_deref() == Some(name.as_str()) {
        let _ = fs::remove_file(active_file()?);
    }
    Ok(())
}

#[tauri::command(async)]
fn switch_profile(name: String, restart: bool) -> Result<(), String> {
    let _op = op_lock();
    let mut target = load_profile(&name)?;
    if !has_valid_token(&parse_stored(&target.token_json, "目标档案")?) {
        return Err("目标档案没有登录凭据，请重新登录并保存".into());
    }
    if winsec::MAX_BLOB < target.token_json.len() {
        return Err("档案凭据超过 Windows 单条凭据长度上限".into());
    }

    require_saved(&get_state()?, "切换")?;
    if adc_enabled() && target.adc.is_none() {
        return Err("Antigravity 已开启 ADC 登录，启动时会优先用 ADC 账号，切换到不含 ADC 的档案不会生效。请先在 Antigravity 中关闭 ADC 再切换".into());
    }

    // 1. 停止进程，避免旧进程把刷新后的 token 回写覆盖
    if restart {
        require_app()?;
        startup_log("switch: stopping Antigravity");
        kill_app()?;
    } else {
        let procs = app_procs()?;
        if procs.locked {
            return Err(LOCKED_MSG.into());
        }
        if procs.running() {
            return Err("Antigravity 仍在运行，请开启自动重启或先关闭应用再切换".into());
        }
    }

    let result = (|| {
        // 2. 根据实际登录身份回存，绝不把手动登录的 B 账号覆盖进 A 档案。
        if let Some(old) = backup_live()? {
            if old.name == name {
                target = old;
            }
        }

        // 3. 写入目标凭据，并清理会造成"双数据源"的残留
        winsec::cred_write(TARGET, USER, target.token_json.as_bytes())?;
        clear_fallback()?;
        if let Some(adc) = &target.adc {
            let p = adc_file()?;
            if let Some(dir) = p.parent() {
                fs::create_dir_all(dir).map_err(es)?;
            }
            fs::write(p, adc).map_err(es)?;
        }
        set_active(&name)?;
        Ok(())
    })();
    startup_log("switch: credential update finished; completing restart");
    finish_restart(result, restart, launch_app)
}

/// 把当前实际登录（含刷新/轮换后的 token）回存到与之身份一致的档案。
/// 返回被更新的档案；当前登录与任何档案都不匹配时不写入。
fn backup_live() -> Result<Option<Profile>, String> {
    let (Some(active), Some(live)) = (get_state()?.active, read_live()?) else {
        return Ok(None);
    };
    let mut old = load_profile(&active)?;
    if !same_login(
        &parse_stored(&old.token_json, "档案")?,
        &parse_stored(&live, "当前登录")?,
    ) {
        return Ok(None);
    }
    old.token_json = live;
    old.saved_at = now();
    if old.adc.is_some() {
        old.adc = fs::read_to_string(adc_file()?).ok().or(old.adc);
    }
    store_profile(&old)?;
    Ok(Some(old))
}

fn remove_if_exists(p: PathBuf) -> Result<(), String> {
    match fs::remove_file(&p) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("删除 {} 失败: {e}", p.display()))
        }
        _ => Ok(()),
    }
}

/// 清理降级文件与 Keyring 超时标记，保证只有凭据管理器一个数据源。
fn clear_fallback() -> Result<(), String> {
    remove_if_exists(fallback_file()?)?;
    remove_if_exists(keyring_marker()?)
}

/// ADC 模式在启动时优先于 Keyring，开启时清空凭据也会被 ADC 账号自动登录。
fn truthy(v: &str) -> bool {
    let v = v.trim();
    !v.is_empty()
        && !["0", "false", "no", "off"]
            .iter()
            .any(|x| v.eq_ignore_ascii_case(x))
}
fn adc_enabled() -> bool {
    if ["AGY_ADC_AUTH", "AGY_CDE_SEAMLESS_AUTH"]
        .iter()
        .any(|k| std::env::var(k).is_ok_and(|v| truthy(&v)))
    {
        return true;
    }
    let Ok(dir) = gemini_dir() else { return false };
    fs::read_to_string(dir.join("config").join("config.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(s.trim_start_matches('\u{feff}')).ok())
        .and_then(|v| v["user_settings"]["enable_adc"].as_bool())
        .unwrap_or(false)
}

/// 登录新账号：回存当前账号后只清除本机凭据并重启 Antigravity。
/// 不调用 Antigravity 的退出登录，因此不会吊销已保存档案里的 token。
#[tauri::command(async)]
fn login_new() -> Result<(), String> {
    let _op = op_lock();
    require_saved(&get_state()?, "登录新账号")?;
    if adc_enabled() {
        return Err("Antigravity 已开启 ADC 登录，清除凭据后仍会自动用 ADC 账号登录。请先在 Antigravity 中关闭 ADC 再试".into());
    }
    require_app()?;

    kill_app()?;
    let result = (|| {
        // 进程退出后再回存一次，拿到最后刷新的 token。
        backup_live()?;
        winsec::cred_delete(TARGET)?;
        clear_fallback()?;
        if read_live()?.is_some() {
            return Err("本地凭据未能清除，Antigravity 未重启".into());
        }
        let _ = fs::remove_file(active_file()?);
        Ok(())
    })();
    finish_restart(result, true, launch_app)
}

#[tauri::command(async)]
fn launch() -> Result<(), String> {
    launch_app()
}

fn startup_log(message: &str) {
    // 同时支持显式诊断输出，目录初始化失败时仍能保留原因。
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--startup-log") {
        if let Some(path) = args.get(2) {
            if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(file, "{} {message}", now());
            }
        }
    }
    if let Ok(base) = env_path("LOCALAPPDATA") {
        let dir = base.join("agy-switch");
        if fs::create_dir_all(&dir).is_ok() {
            if let Ok(mut file) = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("startup.log"))
            {
                let _ = writeln!(file, "{} {message}", now());
            }
        }
    }
}

fn webview_busy(error: &str) -> bool {
    error.to_ascii_lowercase().contains("0x800700aa")
}

fn retry_webview<T>(
    mut create: impl FnMut() -> Result<T, String>,
    mut wait: impl FnMut(),
) -> Result<T, String> {
    for attempt in 1..=4 {
        match create() {
            Ok(window) => return Ok(window),
            Err(error) if webview_busy(&error) && attempt < 4 => {
                startup_log(&format!(
                    "webview busy attempt={attempt}; waiting before retry"
                ));
                wait();
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}

fn webview_error(cache: &std::path::Path, error: &str) -> String {
    let hint = if webview_busy(error) {
        "WebView2 界面缓存仍被占用，等待重试后仍未释放。请完全退出 AGY Switch 后重开；如果仍失败，请重启 Windows。"
    } else {
        "请查看启动日志中的具体错误。如果确实未安装 Microsoft Edge WebView2 Runtime，可从微软官网安装：https://go.microsoft.com/fwlink/p/?LinkId=2124703"
    };
    format!(
        "创建界面失败，缓存目录 {}: {error}\n\n{hint}",
        cache.display()
    )
}

fn main() {
    // 可在真实用户环境运行的只读诊断；不输出邮箱、token 或档案内容。
    let args: Vec<String> = std::env::args().collect();
    // 发布脚本只检查实际启动权限，不创建 WebView2、不读取账号数据。
    if args.get(1).map(String::as_str) == Some("--check-integrity") {
        std::process::exit(
            if winsec::integrity_level().is_ok_and(|level| level >= 0x2000) {
                0
            } else {
                1
            },
        );
    }
    if args.get(1).map(String::as_str) == Some("--diagnose") {
        let mut report = match get_state() {
            Ok(s) => serde_json::json!({
                "credential_target": TARGET, "has_login": s.current.is_some(),
                "email_present": s.current.as_ref().is_some_and(|c| !c.email.is_empty()),
                "auth_method": s.current.as_ref().map(|c| &c.auth_method),
                "profile_count": s.profiles.len(), "matched_profile": s.active.is_some(),
                "app_running": s.app_running,
                "process_error": s.process_error,
                "current_error": s.current_error,
                "profile_error_count": s.profile_errors.len(),
                "elevated": winsec::is_elevated().ok(),
            }),
            Err(e) => serde_json::json!({"error": e}),
        };
        report["local_app_data"] = serde_json::json!(std::env::var_os("LOCALAPPDATA"));
        report["elevated"] = serde_json::json!(winsec::is_elevated().ok());
        report["integrity_level"] = serde_json::json!(winsec::integrity_level().ok());
        if let Some(path) = args.get(2) {
            fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).expect("写入诊断失败");
        }
        return;
    }
    std::panic::set_hook(Box::new(|info| {
        let message = format!("{info}\n\n日志：%LOCALAPPDATA%\\agy-switch\\startup.log");
        startup_log(&message);
        winsec::show_error(&message);
    }));
    // 构建沙箱产物可能带 Low 标签。此时提前说明原因，避免 WebView2 报 ERROR_BUSY。
    let integrity = match winsec::integrity_level() {
        Ok(level) => level,
        Err(error) => {
            startup_log(&error);
            winsec::show_error(&error);
            return;
        }
    };
    startup_log(&format!(
        "starting pid={} integrity=0x{integrity:X}",
        std::process::id()
    ));
    if integrity < 0x2000 {
        let message = "当前 AGY Switch 以低完整性级别运行，无法初始化 WebView2 或访问正常用户数据。\n\n请运行已发布的 %LOCALAPPDATA%\\Programs\\agy-switch\\agy-switch.exe；开发构建请在普通 PowerShell 中运行 scripts\\publish.ps1。分享版请将 ZIP 解压到新文件夹后运行。";
        startup_log(message);
        winsec::show_error(message);
        return;
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            startup_log("duplicate launch: focusing existing window");
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let elevated = winsec::is_elevated()?;
            // WebView2 无法跨完整性级别复用同一个用户数据目录（0x800700AA）。
            // 界面缓存分开；DPAPI 账号档案仍放在原来的 profiles 目录。
            let cache = env_path("LOCALAPPDATA")?
                .join("agy-switch")
                .join(if elevated {
                    "webview-admin"
                } else {
                    "webview-user"
                });
            startup_log(&format!(
                "starting elevated={elevated} cache={}",
                cache.display()
            ));
            fs::create_dir_all(&cache)
                .map_err(|e| format!("无法创建界面缓存 {}: {e}", cache.display()))?;
            startup_log("cache directory ready");
            retry_webview(
                || {
                    tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])
                        .map_err(|e| format!("读取窗口配置失败: {e}"))?
                        .data_directory(cache.clone())
                        .build()
                        .map_err(|e| e.to_string())
                },
                || thread::sleep(Duration::from_secs(1)),
            )
            .map_err(|e| webview_error(&cache, &e))?;
            startup_log("window ready");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            save_profile,
            delete_profile,
            switch_profile,
            login_new,
            launch
        ])
        .run(tauri::generate_context!())
        .expect("启动 AGY Switch 失败");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn busy_webview_recovers_when_old_browser_exits() {
        let mut attempts = 0;
        let mut waits = 0;
        let result = retry_webview(
            || {
                attempts += 1;
                if attempts < 3 {
                    Err("HRESULT(0x800700AA)".into())
                } else {
                    Ok(())
                }
            },
            || waits += 1,
        );
        assert!(result.is_ok());
        assert_eq!((attempts, waits), (3, 2));
    }

    #[test]
    fn busy_webview_retries_are_bounded_and_other_errors_fail_immediately() {
        for (error, expected) in [("HRESULT(0x800700aa)", 4), ("Access denied 0x80070005", 1)] {
            let mut attempts = 0;
            let mut waits = 0;
            let result: Result<(), String> = retry_webview(
                || {
                    attempts += 1;
                    Err(error.into())
                },
                || waits += 1,
            );
            assert_eq!(result.unwrap_err(), error);
            assert_eq!((attempts, waits), (expected, expected - 1));
        }
        let message = webview_error(std::path::Path::new("cache"), "HRESULT(0x800700AA)");
        assert!(message.contains("仍被占用"));
        assert!(!message.contains("安装"));
    }

    fn personal(email: &str, refresh: &str) -> Value {
        json!({"id_token_email": email, "auth_method": "consumer",
            "token": {"access_token": "test-access", "refresh_token": refresh}})
    }

    fn saved_personal() -> Profile {
        Profile {
            name: "saved-account".into(),
            saved_at: 1,
            token_json: personal("test@example.com", "test-refresh").to_string(),
            adc: None,
        }
    }

    #[test]
    fn missing_app_process_is_a_normal_stopped_state() {
        let missing_root = std::env::temp_dir().join(format!("agy-no-install-{}", now()));
        let processes = app_procs_at(&missing_root).expect("no matching process must not fail");
        assert!(processes.pids.is_empty());
    }

    #[test]
    fn process_query_failure_does_not_hide_saved_accounts() {
        let profile = saved_personal();
        let live = profile.token_json.clone();
        let state = build_state(
            Ok(Some(live)),
            vec![Ok(profile)],
            None,
            Err("query failed".into()),
        );
        assert_eq!(state.profiles.len(), 1);
        assert_eq!(state.active.as_deref(), Some("saved-account"));
        assert_eq!(state.process_error.as_deref(), Some("query failed"));
    }

    #[test]
    fn unreadable_current_login_keeps_archives_visible_and_blocks_switch() {
        let state = build_state(
            Err("credential unavailable".into()),
            vec![Ok(saved_personal())],
            None,
            Ok(AppProcs {
                pids: vec![],
                locked: false,
            }),
        );
        assert_eq!(state.profiles.len(), 1);
        assert!(state.current.is_none());
        assert!(require_saved(&state, "切换")
            .unwrap_err()
            .contains("未执行"));
    }

    #[test]
    fn damaged_archive_does_not_hide_other_accounts() {
        let mut broken = saved_personal();
        broken.token_json = "not-json".into();
        let state = build_state(
            Ok(None),
            vec![
                Ok(saved_personal()),
                Ok(broken),
                Err("cannot decrypt".into()),
            ],
            None,
            Ok(AppProcs {
                pids: vec![],
                locked: false,
            }),
        );
        assert_eq!(state.profiles.len(), 1);
        assert_eq!(state.profile_errors.len(), 2);
    }

    #[test]
    fn stopped_app_restarts_even_if_credential_update_fails() {
        let mut launched = false;
        let error = finish_restart(Err("write failed".into()), true, || {
            launched = true;
            Ok(())
        })
        .unwrap_err();
        assert!(launched);
        assert!(error.contains("write failed"));
        assert!(error.contains("已重新打开"));
        assert_eq!(
            finish_restart(Ok(()), true, || Err("start failed".into())).unwrap_err(),
            "start failed"
        );
        assert!(finish_restart(Ok(()), false, || panic!("restart disabled")).is_ok());
    }

    #[test]
    #[ignore = "opens the installed Antigravity and reselects the current saved account; run explicitly in the normal user session"]
    fn installed_app_switches_current_account_and_restarts() {
        assert!(winsec::integrity_level().unwrap() >= 0x2000);
        assert!(
            !app_procs().unwrap().running(),
            "close Antigravity before this test"
        );
        let before = get_state().unwrap();
        assert!(before.current_error.is_none());
        assert!(before.profile_errors.is_empty());
        let name = before.active.expect("current login must already be saved");
        let live_before = parse_stored(&read_live().unwrap().unwrap(), "当前登录").unwrap();
        launch_app().expect("initial app launch");
        switch_profile(name.clone(), true).expect("switch/restart current saved account");
        let after = get_state().unwrap();
        assert!(after.app_running);
        assert!(after.process_error.is_none());
        assert_eq!(after.profiles.len(), before.profiles.len());
        assert_eq!(after.active.as_deref(), Some(name.as_str()));
        let live_after = parse_stored(&read_live().unwrap().unwrap(), "当前登录").unwrap();
        assert!(
            same_login(&live_before, &live_after),
            "account identity changed"
        );
    }

    #[test]
    fn manual_second_login_must_not_overwrite_first_profile() {
        assert!(!same_login(
            &personal("a@example.com", "refresh-a"),
            &personal("b@example.com", "refresh-b")
        ));
    }

    #[test]
    fn rotated_tokens_still_match_same_personal_account() {
        let mut a = personal("A@example.com", "old");
        a["auth_method"] = json!("");
        assert!(same_login(&a, &personal("a@example.com", "new")));
    }

    #[test]
    fn projects_and_auth_modes_are_separate_profiles() {
        let a = personal("a@example.com", "same");
        let mut b = a.clone();
        b["auth_method"] = json!("gcp");
        assert!(!same_login(&a, &b));
        let mut c = b.clone();
        b["project_id"] = json!("project-a");
        c["project_id"] = json!("project-b");
        assert!(!same_login(&b, &c));
    }

    #[test]
    fn unknown_email_requires_matching_refresh_token() {
        assert!(same_login(&personal("", "same"), &personal("", "same")));
        assert!(!same_login(&personal("", "a"), &personal("", "b")));
        // 都没有 refresh token：只有 access token 完全相同才算同一个登录
        let mut other = personal("", "");
        other["token"]["access_token"] = json!("another-access");
        assert!(!same_login(&personal("", ""), &other));
        assert!(same_login(&personal("", ""), &personal("", "")));
    }

    #[test]
    fn logout_residue_never_matches_an_account() {
        let residue = json!({"saved_wif_provider": "provider", "token": null});
        assert!(!has_valid_token(&residue));
        assert!(!same_login(&residue, &residue));
    }

    #[test]
    fn parse_bom_and_keep_credentials_out_of_errors() {
        assert!(parse_stored("\u{feff}{\"token\":null}", "测试").is_ok());
        let error = parse_stored("secret-token-invalid-json", "测试").unwrap_err();
        assert!(!error.contains("secret-token"));
    }

    #[test]
    fn display_icon_parses_to_exe_path() {
        assert_eq!(
            icon_exe(r#""D:\Apps\antigravity\Antigravity.exe",0"#),
            Some(PathBuf::from(r"D:\Apps\antigravity\Antigravity.exe"))
        );
        assert_eq!(
            icon_exe(r"C:\Users\x\AppData\Local\Programs\antigravity\Antigravity.exe,0"),
            Some(PathBuf::from(
                r"C:\Users\x\AppData\Local\Programs\antigravity\Antigravity.exe"
            ))
        );
        assert_eq!(
            icon_exe(r"D:\a,b\Antigravity.exe"),
            Some(PathBuf::from(r"D:\a,b\Antigravity.exe"))
        );
        assert_eq!(icon_exe("  "), None);
    }

    #[test]
    fn adc_env_values() {
        assert!(truthy("1") && truthy("true") && truthy("yes"));
        assert!(!truthy("") && !truthy("0") && !truthy("FALSE") && !truthy(" "));
        assert!(!truthy("no") && !truthy("Off"));
    }

    #[test]
    fn unsaved_login_blocks_switch_and_login_new() {
        let summary = summarize(&personal("a@example.com", "r"));
        let state = |current: Option<Summary>, active: Option<String>| State {
            current,
            profiles: vec![],
            active,
            app_running: false,
            identifiable: true,
            ..Default::default()
        };
        let err = require_saved(&state(Some(summary.clone()), None), "切换").unwrap_err();
        assert!(err.contains("a@example.com"));
        assert!(require_saved(&state(Some(summary), Some("A".into())), "切换").is_ok());
        assert!(require_saved(&state(None, None), "切换").is_ok());
    }

    #[test]
    fn access_token_only_login_is_matchable_but_never_blocks() {
        let login = json!({"auth_method": "gcp", "token": {"access_token": "at-1"}});
        assert!(same_login(&login, &login.clone()));
        let refreshed = json!({"auth_method": "gcp", "token": {"access_token": "at-2"}});
        assert!(!same_login(&login, &refreshed));
        assert!(!identifiable(&login));
        let st = State {
            current: Some(summarize(&login)),
            profiles: vec![],
            active: None,
            app_running: false,
            identifiable: identifiable(&login),
            ..Default::default()
        };
        assert!(require_saved(&st, "切换").is_ok());
        assert!(identifiable(&personal("", "refresh")) && identifiable(&personal("a@x.com", "")));
    }
}
