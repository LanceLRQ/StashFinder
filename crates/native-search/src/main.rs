#![windows_subsystem = "windows"]

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    // The standalone Cargo target is the launcher for the development backend.
    let result = std::env::current_exe()
        .map_err(anyhow::Error::from)
        .and_then(|exe| {
            stashfinder_native_search::run(&exe.with_file_name("stashfinder-service.exe"), &args)
        });
    if let Err(error) = result {
        stashfinder_windows::portable::show_error(&format!("{error:#}"));
        std::process::exit(1);
    }
}
