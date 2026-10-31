#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--internal-parser-worker"] {
        if kaijucrawl::parser::runtime::worker().is_err() {
            std::process::exit(1);
        }
        return;
    }
    if args == ["--version"] {
        println!("KaijuCrawl {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args == ["--help"] {
        println!(
            "KaijuCrawl — DriveThruRPG desktop catalogue\n\nLaunch kaijucrawl to open the desktop dashboard.\nExisting data is used directly from your home folder/.kaijucrawl.\n\nOptions: --help, --version"
        );
        return;
    }
    if !args.is_empty() {
        eprintln!("Unknown option. Use --help or launch the interactive dashboard.");
        std::process::exit(2);
    }
    #[cfg(feature = "desktop")]
    if let Err(e) = kaijucrawl::desktop::run() {
        eprintln!("{}", kaijucrawl::telemetry::display(&e.to_string()));
        std::process::exit(1);
    }
    #[cfg(not(feature = "desktop"))]
    {
        eprintln!("This backend test build has no desktop UI. Build with the default features.");
        std::process::exit(2);
    }
}
