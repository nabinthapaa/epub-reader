//! The desktop binary: everything else lives in the library, which the Android
//! entry point shares.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A path on the command line opens straight away.
    let book = std::env::args_os().nth(1).map(PathBuf::from);
    folio::run(book, None)
}
