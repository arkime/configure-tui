//! Thin binary: parse args, run pre-flight guards, set up/tear down the
//! terminal (with a panic hook that restores it), and run the wizard.

use anyhow::Result;
use arkime_setup::app::{self, App};
use arkime_setup::domain::BuildConfig;
use arkime_setup::guards::{self, GuardOutcome};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::stdout;

struct Cli {
    build: BuildConfig,
    /// Skip the root check — useful for a dry look on a dev box.
    no_root_check: bool,
}

fn parse_args() -> Result<Option<Cli>> {
    let mut build = BuildConfig::defaults();
    let mut no_root_check = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("arkime-setup {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--install-dir" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--install-dir needs a value"))?;
                build.install_dir = v.into();
            }
            "--name" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--name needs a value"))?;
                build.name = v;
            }
            "--no-root-check" => no_root_check = true,
            other => anyhow::bail!("unknown argument: {other} (try --help)"),
        }
    }
    Ok(Some(Cli {
        build,
        no_root_check,
    }))
}

fn print_help() {
    println!(
        "arkime-setup — configure an Arkime installation (native or docker)\n\n\
         USAGE:\n    arkime-setup [OPTIONS]\n\n\
         OPTIONS:\n\
         \x20   --install-dir <PATH>   Override the install dir (default /opt/arkime)\n\
         \x20   --name <NAME>          Override the product name (default arkime)\n\
         \x20   --no-root-check        Do not require root (dev only)\n\
         \x20   -h, --help             Show this help\n\
         \x20   -V, --version          Show version"
    );
}

fn main() -> Result<()> {
    let cli = match parse_args()? {
        Some(c) => c,
        None => return Ok(()),
    };

    let platform = match guards::preflight(!cli.no_root_check) {
        GuardOutcome::Ok(p) => p,
        GuardOutcome::Refuse(msg) => {
            eprintln!("{msg}");
            std::process::exit(1);
        }
    };

    let mut terminal = setup_terminal()?;
    let app = App::new(cli.build, platform);
    let result = app::run(&mut terminal, app);
    restore_terminal()?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;

    // Restore the terminal even if we panic mid-draw.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        default_hook(info);
    }));

    Ok(Terminal::new(CrosstermBackend::new(stdout()))?)
}

fn restore_terminal() -> Result<()> {
    let _ = stdout().execute(LeaveAlternateScreen);
    disable_raw_mode()?;
    Ok(())
}
