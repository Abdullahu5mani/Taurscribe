// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `taurscribe mcp`: read-only transcript server for LLM apps (stdio), no window.
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        taurscribe_lib::mcp_server::run();
        return;
    }
    taurscribe_lib::run()
}
