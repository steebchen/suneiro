#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().nth(1).as_deref() == Some(suneiro_core::delegation_bridge::MCP_FLAG) {
        if let Err(error) = suneiro_core::delegation_bridge::run_stdio() {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        return;
    }
    suneiro_desktop_lib::run()
}
