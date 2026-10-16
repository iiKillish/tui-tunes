#[macro_use]
extern crate serde;

use std::{path::PathBuf, process::exit};

use application::{Application, setup_logging};
use config::set_configuration_base_path;
use log::error;
use ncspot::program_arguments;

mod application;
mod authentication;
mod cli;
mod command;
mod config;
mod events;
mod library;
mod lyrics;
mod model;
mod pagination;
mod panic;
mod queue;
mod serialization;
mod sharing;
mod spotify;
mod spotify_api;
mod spotify_url;
mod spotify_worker;
mod theme;
mod traits;
mod tui;
mod utils;

#[cfg(unix)]
mod ipc;

#[cfg(feature = "mpris")]
mod mpris;

fn main() -> Result<(), String> {
    // Set a custom backtrace hook that writes the backtrace to a file instead of stdout, since
    // stdout is in use by the TUI.
    panic::register_backtrace_panic_handler();

    // Parse the command line arguments.
    let matches = program_arguments().get_matches();

    // Enable debug logging to a file if specified on the command line.
    if let Some(filename) = matches.get_one::<PathBuf>("debug") {
        setup_logging(filename).expect("logger could not be initialized");
    }

    // Set the configuration base path. All configuration files are read/written relative to this
    // path.
    set_configuration_base_path(matches.get_one::<PathBuf>("basepath").cloned());

    match matches.subcommand() {
        Some(("info", _subcommand_matches)) => cli::info(),
        Some((_, _)) => unreachable!(),
        None => {
            // Export the environment variables derived from the configuration
            // file (proxy, PulseAudio stream properties). This must happen
            // while the process is still single-threaded, i.e. before the
            // tokio runtime and its worker threads are created in
            // `Application::new`: `set_var` is not thread safe.
            config::preload_environment(matches.get_one::<String>("config").cloned());

            // Create the application.
            let mut application =
                match Application::new(matches.get_one::<String>("config").cloned()) {
                    Ok(application) => application,
                    Err(error) => {
                        eprintln!("{error}");
                        error!("{error}");
                        exit(-1);
                    }
                };

            // Start the application event loop.
            application.run()
        }
    }?;

    Ok(())
}
