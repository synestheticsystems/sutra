use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "sutra", about = "Dev environment status & orchestration")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Monitor dev environments (GUI or TUI dashboard)
    Mon {
        /// Force TUI mode (terminal UI) even if GUI is available
        #[arg(long)]
        tui: bool,

        /// Keep attached to the terminal (don't background the GUI)
        #[arg(long)]
        foreground: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Command::Mon {
        tui: false,
        foreground: false,
    }) {
        Command::Mon { tui, foreground } => {
            if tui {
                #[cfg(feature = "tui")]
                sutra::tui::run();

                #[cfg(not(feature = "tui"))]
                {
                    eprintln!("TUI not available (compiled without 'tui' feature)");
                    std::process::exit(1);
                }
            } else {
                // GUI mode: background by default so the shell is freed.
                // When launched from a macOS .app bundle there is no terminal
                // to free, and forking would orphan the GUI from the bundle
                // (Launch Services would see the main process exit), so run in
                // the foreground in that case.
                if !foreground && !running_in_app_bundle() {
                    daemonize();
                }

                #[cfg(feature = "gui")]
                sutra::gui::run();

                #[cfg(all(not(feature = "gui"), feature = "tui"))]
                sutra::tui::run();

                #[cfg(all(not(feature = "gui"), not(feature = "tui")))]
                {
                    eprintln!("No UI available (compiled without 'tui' or 'gui' features)");
                    std::process::exit(1);
                }
            }
        }
    }
}

/// Returns true when this executable is running from inside a macOS `.app`
/// bundle (its path contains `.app/Contents/MacOS/`). In that case there is no
/// controlling terminal to free and forking would orphan the GUI process from
/// the bundle, so the caller should run in the foreground instead.
fn running_in_app_bundle() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.into_os_string().into_string().ok())
        .is_some_and(|p| p.contains(".app/Contents/MacOS/"))
}

/// Fork the process and exit the parent so the GUI runs detached from the terminal.
fn daemonize() {
    use std::env;
    use std::process::Command;

    // Re-exec ourselves with --foreground to prevent infinite fork loop
    let exe = env::current_exe().expect("could not determine executable path");
    let args: Vec<String> = env::args().skip(1).collect();

    let mut cmd = Command::new(exe);
    // If no subcommand was given (bare `sutra`), inject `mon`
    if args.is_empty() || (!args.iter().any(|a| a == "mon")) {
        cmd.arg("mon");
    }
    for arg in &args {
        cmd.arg(arg);
    }
    cmd.arg("--foreground");

    // Detach: redirect stdin/stdout/stderr to null, start in background
    use std::process::Stdio;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    match cmd.spawn() {
        Ok(_) => std::process::exit(0), // parent exits, child runs the GUI
        Err(e) => {
            eprintln!("failed to background: {e}");
            // Fall through — run in foreground instead
        }
    }
}
