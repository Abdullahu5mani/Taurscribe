// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `taurscribe mcp`: read-only transcript server for LLM apps (stdio), no window.
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        taurscribe_lib::mcp_server::run();
        return;
    }
    // `taurscribe <command>`: the command-line tool (start, stop, transcribe, history …).
    // The app itself only gets flags (`--autostart`), so any other word is a
    // mistyped command: report it instead of opening the app.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| taurscribe_lib::cli::is_cli_command(a) || !a.starts_with('-')) {
        std::process::exit(taurscribe_lib::cli::run(args));
    }
    taurscribe_lib::run()
}
