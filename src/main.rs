mod cli;
mod input;
mod kitty;
mod runtime;
mod terminal;

use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    terminal::install_panic_restore_hook();

    let args = cli::Args::parse();
    let asset_dir = cli::resolve_asset_dir(args.asset_dir)?;
    let doom_argv = cli::build_doom_argv(&asset_dir, args.doom_args);
    let mut c_args = cli::to_c_args(&doom_argv)?;

    let sound_dir = asset_dir.join("sound");
    // The C engine is single-threaded at this point; set the sound path before
    // miniaudio can start any callback threads.
    unsafe {
        std::env::set_var("KITDOOM_SOUND_DIR", &sound_dir);
    }

    runtime::reset_exit_request();
    runtime::install_signal_handlers()?;

    let _terminal = terminal::TerminalSession::enter()?;
    runtime::run(&mut c_args)
}
