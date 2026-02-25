// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(all(feature = "gui", feature = "team"))]
    mycelica_lib::run_team();

    #[cfg(all(feature = "gui", not(feature = "team")))]
    mycelica_lib::run();
}
