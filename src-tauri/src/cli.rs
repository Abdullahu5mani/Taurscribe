//! The `taurscribe` command-line tool.
//!
//! Dictation and file transcription go to the running app through the local
//! endpoint in `cli_server.rs` (the app is started if needed). History commands
//! read the transcript database directly, so they work while the app is closed.

use serde::Serialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::cli_server::{endpoint_path, CliResponse, Endpoint};

const HELP: &str = "\
taurscribe — speech-to-text from the command line

Usage: taurscribe <command> [options]

Dictation (uses the running app; starts it if needed):
  start                    start dictating, like pressing the hotkey
  stop [--print]           stop; the text is pasted as usual. --print also
                           waits for the transcript and prints it
  toggle                   start or stop

Files:
  transcribe <file>... [--json]
                           transcribe audio files with the active engine

Status:
  status [--json]          is the app running, recording, which model is loaded

History (reads your transcripts; works with the app closed):
  history [--limit N] [--kind dictation|file|meeting]
  search <words>... [--limit N] [--kind dictation|file|meeting]
  show <id>                one dictation or file transcript
  meeting <id>             one meeting, with speaker labels

Setup:
  install-cli              put `taurscribe` on your PATH
  uninstall-cli            remove it again
  mcp                      run the read-only MCP server for LLM apps (stdio)
  help, --version
";

const EXIT_ERROR: i32 = 1;
const EXIT_USAGE: i32 = 2;
const EXIT_NOT_RUNNING: i32 = 3;

/// True when `taurscribe <arg>` should run the CLI instead of opening the app.
pub fn is_cli_command(arg: &str) -> bool {
    matches!(
        arg,
        "start" | "stop" | "toggle" | "transcribe" | "status" | "history" | "search" | "show" | "meeting"
            | "install-cli" | "uninstall-cli" | "help" | "--help" | "-h" | "--version" | "-V"
    )
}

/// Runs a CLI command and returns the process exit code.
pub fn run(args: Vec<String>) -> i32 {
    attach_console();
    let (cmd, rest) = match args.split_first() {
        Some((c, r)) => (c.as_str(), r.to_vec()),
        None => ("help", Vec::new()),
    };
    let o = Opts::parse(&rest);
    let result = match cmd {
        "help" | "--help" | "-h" => {
            print!("{HELP}");
            Ok(())
        }
        "--version" | "-V" => {
            println!("taurscribe {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "status" => cmd_status(&o),
        "start" | "stop" | "toggle" => cmd_dictate(cmd, &o),
        "transcribe" => cmd_transcribe(&o),
        "history" => cmd_history(&o),
        "search" => cmd_search(&o),
        "show" => cmd_show("get_transcript", &o),
        "meeting" => cmd_show("get_meeting", &o),
        "install-cli" | "uninstall-cli" => o.check_bare().and_then(|()| {
            let r = if cmd == "install-cli" { install() } else { uninstall() };
            r.map(|m| println!("{m}")).map_err(Fail::Other)
        }),
        other => Err(Fail::Usage(format!("unknown command: {other}"))),
    };
    match result {
        Ok(()) => 0,
        Err(Fail::Usage(m)) => {
            eprintln!("taurscribe: {m}\nRun `taurscribe help` for the list of commands.");
            EXIT_USAGE
        }
        Err(Fail::NotRunning) => {
            eprintln!("taurscribe: the app isn't running. Open Taurscribe, or run `taurscribe start` to launch it.");
            EXIT_NOT_RUNNING
        }
        Err(Fail::Other(m)) => {
            eprintln!("taurscribe: {m}");
            EXIT_ERROR
        }
    }
}

enum Fail {
    Usage(String),
    NotRunning,
    Other(String),
}

impl From<String> for Fail {
    fn from(s: String) -> Self {
        Fail::Other(s)
    }
}

/// Positional arguments plus the few flags the commands take.
struct Opts {
    pos: Vec<String>,
    json: bool,
    print: bool,
    limit: Option<i64>,
    kind: Option<String>,
    bad: Option<String>,
}

impl Opts {
    fn parse(args: &[String]) -> Self {
        let mut o = Opts { pos: Vec::new(), json: false, print: false, limit: None, kind: None, bad: None };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--json" => o.json = true,
                "--print" => o.print = true,
                "--limit" | "-n" => match it.next().and_then(|v| v.parse().ok()) {
                    Some(n) => o.limit = Some(n),
                    None => o.bad = Some("--limit needs a number".into()),
                },
                "--kind" => match it.next().map(String::as_str) {
                    Some(k @ ("dictation" | "file" | "meeting")) => o.kind = Some(k.into()),
                    _ => o.bad = Some("--kind must be dictation, file or meeting".into()),
                },
                "--" => o.pos.extend(it.by_ref().cloned()),
                f if f.starts_with("--") => o.bad = Some(format!("unknown option: {f}")),
                _ => o.pos.push(a.clone()),
            }
        }
        o
    }
    /// For commands that take no arguments at all.
    fn check_bare(&self) -> Result<(), Fail> {
        self.check()?;
        if self.json || self.print || self.limit.is_some() || self.kind.is_some() || !self.pos.is_empty() {
            return Err(Fail::Usage("this command takes no arguments".into()));
        }
        Ok(())
    }
    fn check(&self) -> Result<(), Fail> {
        match &self.bad {
            Some(m) => Err(Fail::Usage(m.clone())),
            None => Ok(()),
        }
    }
}

// ── talking to the app ──────────────────────────────────────────────────────

fn read_endpoint() -> Option<Endpoint> {
    let s = std::fs::read_to_string(endpoint_path()?).ok()?;
    serde_json::from_str(&s).ok()
}

/// Sends one request. `Err(NotRunning)` when nothing answers on the advertised port.
fn request(cmd: &str, args: Value, timeout: Option<Duration>) -> Result<Value, Fail> {
    let ep = read_endpoint().ok_or(Fail::NotRunning)?;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], ep.port));
    let mut sock = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|_| Fail::NotRunning)?;
    sock.set_read_timeout(timeout).map_err(|e| e.to_string())?;
    let line = json!({ "token": ep.token, "cmd": cmd, "args": args }).to_string() + "\n";
    sock.write_all(line.as_bytes()).map_err(|_| Fail::NotRunning)?;
    let mut reply = String::new();
    BufReader::new(sock).read_line(&mut reply).map_err(|e| format!("no reply from Taurscribe: {e}"))?;
    // Another program on a reused port won't speak this protocol.
    let resp: CliResponse = serde_json::from_str(&reply).map_err(|_| Fail::NotRunning)?;
    if resp.ok {
        Ok(resp.data)
    } else {
        Err(Fail::Other(resp.error.unwrap_or_else(|| "request failed".into())))
    }
}

/// The status, or `None` when the app isn't running.
fn try_status() -> Result<Option<Value>, Fail> {
    match request("status", Value::Null, Some(Duration::from_secs(10))) {
        Ok(v) => Ok(Some(v)),
        Err(Fail::NotRunning) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Starts the app if needed and waits until its UI can receive dictation events.
fn ensure_running() -> Result<(), Fail> {
    if let Some(st) = try_status()? {
        if st["ui_ready"].as_bool() == Some(true) {
            return Ok(());
        }
    } else {
        eprintln!("Starting Taurscribe…");
        launch_app()?;
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(400));
        if let Some(st) = try_status()? {
            if st["ui_ready"].as_bool() == Some(true) {
                return Ok(());
            }
        }
    }
    Err(Fail::Other("Taurscribe did not finish starting within 60 seconds".into()))
}

/// The real executable, with any `install-cli` symlink resolved.
fn app_exe() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("could not find the Taurscribe executable: {e}"))?;
    Ok(std::fs::canonicalize(&exe).unwrap_or(exe))
}

fn launch_app() -> Result<(), String> {
    let exe = app_exe()?;
    // macOS: go through LaunchServices so the app gets its normal bundle context.
    #[cfg(target_os = "macos")]
    if let Some(bundle) = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")) {
        let ok = std::process::Command::new("open").arg("-a").arg(bundle).status().map_err(|e| e.to_string())?.success();
        return if ok { Ok(()) } else { Err("`open` could not start Taurscribe".into()) };
    }
    let mut cmd = std::process::Command::new(&exe);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(DETACHED_PROCESS);
    }
    cmd.spawn().map(|_| ()).map_err(|e| format!("could not start Taurscribe: {e}"))
}

// ── commands ────────────────────────────────────────────────────────────────

fn cmd_status(o: &Opts) -> Result<(), Fail> {
    o.check()?;
    let Some(st) = try_status()? else {
        if o.json {
            println!("{}", json!({ "running": false }));
            return Ok(());
        }
        return Err(Fail::NotRunning);
    };
    if o.json {
        let mut st = st;
        st["running"] = json!(true);
        println!("{st}");
        return Ok(());
    }
    let s = |k: &str| st[k].as_str().unwrap_or("").to_string();
    println!("Taurscribe {} · running (pid {})", s("version"), st["pid"]);
    let rec = if st["recording"].as_bool() == Some(true) {
        if st["paused"].as_bool() == Some(true) { "paused" } else { "yes" }
    } else {
        "no"
    };
    println!("Recording: {rec}");
    let model = match st["model"].as_str() {
        Some(m) => format!("{m} ({})", st["backend"].as_str().unwrap_or("?")),
        None if st["engine_loading"].as_bool() == Some(true) => "loading…".into(),
        None => "not loaded (loads on first use)".into(),
    };
    println!("Engine: {} · model: {model}", s("engine"));
    Ok(())
}

fn cmd_dictate(cmd: &str, o: &Opts) -> Result<(), Fail> {
    o.check()?;
    if o.print && cmd != "stop" {
        return Err(Fail::Usage("--print only works with `stop`".into()));
    }
    // Stopping never launches the app: there is nothing to stop.
    if cmd == "stop" {
        try_status()?.ok_or(Fail::NotRunning)?;
    } else {
        ensure_running()?;
    }
    let before = if o.print { crate::mcp_server::latest_transcript().map(|(id, _)| id) } else { None };
    let r = request("dictate", json!({ "action": cmd }), Some(Duration::from_secs(10)))?;
    let started = r["was_recording"].as_bool() != Some(true);
    match cmd {
        "start" => eprintln!("Recording. Run `taurscribe stop` to finish."),
        "toggle" if started => eprintln!("Recording. Run `taurscribe toggle` again to finish."),
        _ => eprintln!("Stopped. Transcribing…"),
    }
    if o.print {
        // The frontend saves each dictation to history once post-processing is done.
        let deadline = Instant::now() + Duration::from_secs(180);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(300));
            if let Some((id, text)) = crate::mcp_server::latest_transcript() {
                if before.map_or(true, |b| id > b) {
                    println!("{text}");
                    return Ok(());
                }
            }
        }
        return Err(Fail::Other("no transcript was saved (the recording may have been too short, or silent)".into()));
    }
    Ok(())
}

fn cmd_transcribe(o: &Opts) -> Result<(), Fail> {
    o.check()?;
    if o.pos.is_empty() {
        return Err(Fail::Usage("transcribe needs at least one audio file".into()));
    }
    let files: Vec<PathBuf> = o
        .pos
        .iter()
        .map(|f| std::fs::canonicalize(f).map_err(|_| Fail::Other(format!("no such file: {f}"))))
        .collect::<Result<_, _>>()?;
    ensure_running()?;
    let many = files.len() > 1;
    let mut failed = 0;
    for f in &files {
        let shown = f.display().to_string();
        eprintln!("Transcribing {shown}…");
        match request("transcribe", json!({ "path": shown }), None) {
            Ok(r) => {
                if o.json {
                    let mut r = r;
                    r["file"] = json!(shown);
                    println!("{r}");
                } else {
                    if many {
                        println!("== {shown}");
                    }
                    println!("{}", r["transcript"].as_str().unwrap_or("").trim());
                }
            }
            Err(Fail::Other(e)) if many => {
                eprintln!("taurscribe: {shown}: {e}");
                failed += 1;
            }
            Err(e) => return Err(e),
        }
    }
    if failed > 0 {
        return Err(Fail::Other(format!("{failed} of {} file(s) failed", files.len())));
    }
    Ok(())
}

fn tool_args(o: &Opts) -> Value {
    let mut a = json!({});
    if let Some(n) = o.limit {
        a["limit"] = json!(n);
    }
    if let Some(k) = &o.kind {
        a["kind"] = json!(k);
    }
    a
}

fn cmd_history(o: &Opts) -> Result<(), Fail> {
    o.check()?;
    let tool = if o.kind.as_deref() == Some("meeting") { "list_meetings" } else { "list_transcripts" };
    println!("{}", crate::mcp_server::run_tool(tool, &tool_args(o))?);
    Ok(())
}

fn cmd_search(o: &Opts) -> Result<(), Fail> {
    o.check()?;
    if o.pos.is_empty() {
        return Err(Fail::Usage("search needs at least one word".into()));
    }
    let mut a = tool_args(o);
    a["query"] = json!(o.pos.join(" "));
    println!("{}", crate::mcp_server::run_tool("search", &a)?);
    Ok(())
}

fn cmd_show(tool: &str, o: &Opts) -> Result<(), Fail> {
    o.check()?;
    let id: i64 = o
        .pos
        .first()
        .and_then(|s| s.trim_start_matches('#').parse().ok())
        .ok_or_else(|| Fail::Usage("give the id, e.g. `taurscribe show 42`".into()))?;
    println!("{}", crate::mcp_server::run_tool(tool, &json!({ "id": id }))?);
    Ok(())
}

// ── install ─────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct InstallStatus {
    pub installed: bool,
    /// Where the command lives (or would live) on PATH.
    pub location: String,
    /// Shown next to the button, e.g. "~/.local/bin is not on your PATH".
    pub note: Option<String>,
}

#[cfg(target_os = "macos")]
const LINK: &str = "/usr/local/bin/taurscribe";

#[cfg(target_os = "linux")]
fn link_path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".local/bin/taurscribe"))
}

#[cfg(unix)]
fn points_to_us(link: &Path) -> bool {
    match (std::fs::canonicalize(link), app_exe()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

pub fn install_status() -> InstallStatus {
    #[cfg(target_os = "macos")]
    {
        let installed = points_to_us(Path::new(LINK));
        let note = (!installed && Path::new(LINK).exists())
            .then(|| format!("{LINK} already exists and points somewhere else; installing replaces it"));
        InstallStatus { installed, location: LINK.into(), note }
    }
    #[cfg(target_os = "linux")]
    {
        // Distro packages already put the binary in /usr/bin.
        if let Ok(exe) = app_exe() {
            if exe.parent() == Some(Path::new("/usr/bin")) {
                return InstallStatus { installed: true, location: exe.display().to_string(), note: None };
            }
        }
        let link = link_path().unwrap_or_default();
        let on_path = link.parent().is_some_and(dir_on_path);
        InstallStatus {
            installed: points_to_us(&link),
            location: link.display().to_string(),
            note: (!on_path).then(|| "~/.local/bin is not on your PATH; add it in your shell profile".into()),
        }
    }
    #[cfg(windows)]
    {
        let dir = app_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)).unwrap_or_default();
        let installed = windows_user_path().is_some_and(|p| p.split(';').any(|d| same_dir(d, &dir)));
        InstallStatus {
            installed,
            location: dir.display().to_string(),
            note: installed.then(|| "open a new terminal to pick up the PATH change".into()),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    InstallStatus { installed: false, location: String::new(), note: Some("not supported on this platform".into()) }
}

pub fn install() -> Result<String, String> {
    let exe = app_exe()?;
    #[cfg(target_os = "macos")]
    {
        let link = Path::new(LINK);
        let dir = link.parent().unwrap();
        // Homebrew on Intel Macs makes /usr/local/bin user-writable; otherwise ask for admin rights.
        let direct = std::fs::create_dir_all(dir).is_ok() && {
            let _ = std::fs::remove_file(link);
            std::os::unix::fs::symlink(&exe, link).is_ok()
        };
        if !direct {
            let sh = format!(
                "mkdir -p {} && ln -sf {} {}",
                sh_quote(&dir.to_string_lossy()),
                sh_quote(&exe.to_string_lossy()),
                sh_quote(LINK)
            );
            let script = format!(
                "do shell script \"{}\" with prompt \"Taurscribe wants to install the taurscribe command.\" with administrator privileges",
                sh.replace('\\', "\\\\").replace('"', "\\\"")
            );
            let out = std::process::Command::new("osascript").arg("-e").arg(script).output().map_err(|e| e.to_string())?;
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr);
                return Err(if err.contains("-128") { "Cancelled.".into() } else { format!("Could not install: {}", err.trim()) });
            }
        }
        Ok(format!("Installed: {LINK} → {}. Open a new terminal and run `taurscribe help`.", exe.display()))
    }
    #[cfg(target_os = "linux")]
    {
        if exe.parent() == Some(Path::new("/usr/bin")) {
            return Ok("Already on your PATH (installed by the package).".into());
        }
        let link = link_path().ok_or("no home directory")?;
        std::fs::create_dir_all(link.parent().unwrap()).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&exe, &link).map_err(|e| format!("could not create {}: {e}", link.display()))?;
        let mut msg = format!("Installed: {} → {}.", link.display(), exe.display());
        if !link.parent().is_some_and(dir_on_path) {
            msg.push_str(" Add ~/.local/bin to your PATH, then open a new terminal.");
        }
        Ok(msg)
    }
    #[cfg(windows)]
    {
        let dir = exe.parent().ok_or("no install folder")?.to_path_buf();
        let current = windows_user_path().unwrap_or_default();
        if current.split(';').any(|d| same_dir(d, &dir)) {
            return Ok("Already on your PATH. Open a new terminal and run `taurscribe help`.".into());
        }
        let mut parts: Vec<&str> = current.split(';').filter(|s| !s.is_empty()).collect();
        let d = dir.to_string_lossy().to_string();
        parts.push(&d);
        set_windows_user_path(&parts.join(";"))?;
        Ok(format!("Added {} to your PATH. Open a new terminal and run `taurscribe help`.", dir.display()))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = exe;
        Err("not supported on this platform".into())
    }
}

pub fn uninstall() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let link = Path::new(LINK);
        if !points_to_us(link) {
            return Ok("Nothing to remove.".into());
        }
        if std::fs::remove_file(link).is_err() {
            let script = format!(
                "do shell script \"rm -f {}\" with prompt \"Taurscribe wants to remove the taurscribe command.\" with administrator privileges",
                sh_quote(LINK).replace('\\', "\\\\").replace('"', "\\\"")
            );
            let out = std::process::Command::new("osascript").arg("-e").arg(script).output().map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Err(format!("Could not remove {LINK}: {}", String::from_utf8_lossy(&out.stderr).trim()));
            }
        }
        Ok(format!("Removed {LINK}."))
    }
    #[cfg(target_os = "linux")]
    {
        let link = link_path().ok_or("no home directory")?;
        if !points_to_us(&link) {
            return Ok("Nothing to remove.".into());
        }
        std::fs::remove_file(&link).map_err(|e| e.to_string())?;
        Ok(format!("Removed {}.", link.display()))
    }
    #[cfg(windows)]
    {
        let dir = app_exe()?.parent().ok_or("no install folder")?.to_path_buf();
        let current = windows_user_path().unwrap_or_default();
        let kept: Vec<&str> = current.split(';').filter(|d| !d.is_empty() && !same_dir(d, &dir)).collect();
        if kept.len() == current.split(';').filter(|d| !d.is_empty()).count() {
            return Ok("Nothing to remove.".into());
        }
        set_windows_user_path(&kept.join(";"))?;
        Ok(format!("Removed {} from your PATH.", dir.display()))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    Err("not supported on this platform".into())
}

#[cfg(target_os = "macos")]
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(target_os = "linux")]
fn dir_on_path(dir: &Path) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == dir))
}

#[cfg(windows)]
fn same_dir(entry: &str, dir: &Path) -> bool {
    entry.trim().trim_end_matches('\\').eq_ignore_ascii_case(dir.to_string_lossy().trim_end_matches('\\'))
}

/// Runs PowerShell without flashing a console window from the GUI.
#[cfg(windows)]
fn powershell(script: &str) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("could not run PowerShell: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches(['\r', '\n']).to_string())
}

/// The user's own PATH (not the merged system + user value).
#[cfg(windows)]
fn windows_user_path() -> Option<String> {
    powershell("[Environment]::GetEnvironmentVariable('Path','User')").ok()
}

/// Writes the user PATH; .NET broadcasts WM_SETTINGCHANGE so new terminals see it.
#[cfg(windows)]
fn set_windows_user_path(value: &str) -> Result<(), String> {
    let quoted = value.replace('\'', "''");
    powershell(&format!("[Environment]::SetEnvironmentVariable('Path','{quoted}','User')")).map(|_| ())
}

/// Release builds on Windows are GUI-subsystem programs with no console, so
/// output would vanish. Attach to the terminal that started us instead.
fn attach_console() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(a: &[&str]) -> Opts {
        Opts::parse(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn only_known_subcommands_run_the_cli() {
        for c in ["start", "stop", "toggle", "transcribe", "status", "history", "search", "show", "meeting", "install-cli", "help", "--version"] {
            assert!(is_cli_command(c), "{c}");
        }
        // Anything else (no args, macOS -psn_ args, stray paths) still opens the app.
        for c in ["", "-psn_0_12345", "/tmp/a.wav", "mcp", "Start"] {
            assert!(!is_cli_command(c), "{c}");
        }
    }

    #[test]
    fn parses_flags_and_positionals() {
        let o = opts(&["budget", "q3", "--limit", "5", "--kind", "meeting", "--json"]);
        assert_eq!(o.pos, ["budget", "q3"]);
        assert_eq!(o.limit, Some(5));
        assert_eq!(o.kind.as_deref(), Some("meeting"));
        assert!(o.json && !o.print && o.check().is_ok());
    }

    #[test]
    fn rejects_bad_flags() {
        assert!(opts(&["--limit", "x"]).check().is_err());
        assert!(opts(&["--kind", "podcast"]).check().is_err());
        assert!(opts(&["--help"]).check().is_err());
        assert!(opts(&["extra"]).check_bare().is_err());
        assert!(opts(&[]).check_bare().is_ok());
    }

    #[test]
    fn double_dash_keeps_dash_names_as_files() {
        let o = opts(&["--", "--weird.wav", "-x.m4a"]);
        assert_eq!(o.pos, ["--weird.wav", "-x.m4a"]);
        assert!(o.check().is_ok());
    }
}
