use std::error::Error;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use log::{error, info, trace};

#[cfg(unix)]
use signal_hook::{consts::SIGHUP, consts::SIGTERM, iterator::Signals};

use crate::command::Command;
use crate::config::{Config, PlaybackState};
use crate::events::{Event, EventManager};
use crate::library::Library;
use crate::queue::Queue;
use crate::spotify::{PlayerEvent, Spotify};
use crate::tui::page::Ctx;
use crate::{authentication, tui, utils};
use crate::{command, queue, spotify};

#[cfg(feature = "mpris")]
use crate::mpris::MprisManager;

#[cfg(unix)]
use crate::ipc::{self, IpcSocket};

/// Set up the global logger to log to `filename`.
pub fn setup_logging(filename: &Path) -> Result<(), fern::InitError> {
    fern::Dispatch::new()
        // Perform allocation-free log formatting
        .format(|out, message, record| {
            out.finish(format_args!(
                "{} [{}] [{}] {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                record.level(),
                message
            ))
        })
        // Add blanket level filter -
        .level(log::LevelFilter::Debug)
        // Set runtime log level for modules
        .level_for("ttunes", log::LevelFilter::Trace)
        // Output to stdout, files, and other Dispatch configurations
        .chain(fern::log_file(filename)?)
        // Apply globally
        .apply()?;
    Ok(())
}

/// The global Tokio runtime for running asynchronous tasks.
pub static ASYNC_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// The representation of a ttunes application.
pub struct Application {
    /// The music queue which controls playback order.
    queue: Arc<Queue>,
    /// Internally shared
    spotify: Spotify,
    /// Internally shared
    event_manager: EventManager,
    /// An IPC implementation using the D-Bus MPRIS protocol, used to control and inspect ttunes.
    #[cfg(unix)]
    ipc: Option<IpcSocket>,
    /// Handles for the UI.
    ctx: Ctx,
    /// The loading screen, until the UI takes over.
    splash: Option<tui::splash::Splash>,
}

impl Application {
    /// Create a new ttunes application.
    ///
    /// # Arguments
    ///
    /// * `configuration_file_path` - Relative path to the configuration file inside the base path
    pub fn new(configuration_file_path: Option<String>) -> Result<Self, Box<dyn Error>> {
        ASYNC_RUNTIME
            .set(
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .unwrap(),
            )
            .unwrap();

        let configuration = Arc::new(Config::new(configuration_file_path));
        let credentials = authentication::get_credentials(&configuration)?;
        // Before the UI takes over the terminal, which would swallow the answers.
        crate::terminal_colors::detect();
        tui::images::configure(configuration.values().image_protocol.as_deref());
        let palette = configuration.build_theme();

        if let Err(e) = authentication::get_rspotify_token() {
            error!("Failed to get rspotify token: {e}");
        }

        // Without a terminal to take over, say so the plain way.
        let splash = tui::splash::Splash::start(palette, "Connecting to Spotify").ok();
        if splash.is_none() {
            println!("Connecting to Spotify..");
        }

        let event_manager = EventManager::new();
        tui::set_event_manager(event_manager.clone());

        #[cfg_attr(not(feature = "mpris"), allow(unused_mut))]
        let mut spotify =
            spotify::Spotify::new(event_manager.clone(), credentials, configuration.clone())?;

        if let Some(splash) = &splash {
            splash.status("Loading your library");
        }
        let library = Arc::new(Library::new(
            event_manager.clone(),
            spotify.clone(),
            configuration.clone(),
        ));

        let queue = Arc::new(queue::Queue::new(
            spotify.clone(),
            configuration.clone(),
            library.clone(),
        ));

        #[cfg(feature = "mpris")]
        let mpris_manager = MprisManager::new(
            event_manager.clone(),
            queue.clone(),
            library.clone(),
            spotify.clone(),
        );

        #[cfg(feature = "mpris")]
        spotify.set_mpris(mpris_manager.clone());

        // Load the last played track into the player
        let playback_state = configuration.state().playback_state.clone();
        let queue_state = configuration.state().queuestate.clone();

        if let Some(playable) = queue.get_current() {
            spotify.load(
                &playable,
                playback_state == PlaybackState::Playing,
                queue_state.track_progress.as_millis() as u32,
            );
            spotify.update_track();
            match playback_state {
                PlaybackState::Stopped => {
                    spotify.stop();
                }
                PlaybackState::Paused | PlaybackState::Playing | PlaybackState::Default => {
                    spotify.pause();
                }
            }
        }

        #[cfg(unix)]
        let ipc = if let Ok(runtime_directory) = utils::create_runtime_directory() {
            Some(
                ipc::IpcSocket::new(
                    ASYNC_RUNTIME.get().unwrap().handle(),
                    runtime_directory.join("ttunes.sock"),
                    event_manager.clone(),
                )
                .map_err(|e| e.to_string())?,
            )
        } else {
            error!("failed to create IPC socket: no suitable user runtime directory found");
            None
        };

        // Pictures for artist cards that come without one.
        {
            let events = event_manager.clone();
            crate::model::artist::init_image_lookups(spotify.clone(), move || events.trigger());
        }

        let ctx = Ctx {
            queue: queue.clone(),
            library,
            spotify: spotify.clone(),
            config: configuration,
            events: event_manager.clone(),
            palette,
        };

        Ok(Self {
            queue,
            spotify,
            event_manager,
            #[cfg(unix)]
            ipc,
            ctx,
            splash,
        })
    }

    /// Start the application and run the event loop.
    pub fn run(&mut self) -> Result<(), String> {
        #[cfg(unix)]
        let mut signals =
            Signals::new([SIGTERM, SIGHUP]).expect("could not register signal handler");

        drop(self.splash.take());
        let mut terminal = tui::Terminal::start().map_err(|e| e.to_string())?;
        let mut app = tui::app::App::new(self.ctx.clone());

        while !app.quit {
            #[cfg(unix)]
            for signal in signals.pending() {
                if signal == SIGTERM || signal == SIGHUP {
                    info!("Caught {signal}, cleaning up and closing");
                    app.run(&Command::Quit);
                }
            }
            if self.queue.sleep_if_due() {
                app.set_message(Ok("Sleep timer: paused".into()));
            }
            terminal.draw(&mut app).map_err(|e| e.to_string())?;

            // Wait for something to happen, or until something on screen changes with time.
            let mut next = self.event_manager.recv_timeout(app.redraw_after());
            while let Some(event) = next {
                self.handle(event, &mut app, &mut terminal);
                next = self.event_manager.msg_iter().next();
            }
        }
        terminal.stop().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn handle(&mut self, event: Event, app: &mut tui::app::App, terminal: &mut tui::Terminal) {
        match event {
            Event::Player(state) => {
                trace!("event received: {state:?}");
                // Before the status says it's over, which leaves nothing to note.
                if state == PlayerEvent::FinishedTrack {
                    self.queue.record_finished();
                }
                self.spotify.update_status(state.clone());

                #[cfg(unix)]
                if let Some(ref ipc) = self.ipc {
                    ipc.publish(&state, self.queue.get_current());
                }

                if state == PlayerEvent::FinishedTrack {
                    self.queue.next(false);
                }
            }
            Event::Queue(event) => self.queue.handle_event(event),
            Event::SessionDied => {
                if self.spotify.start_worker(None).is_err() {
                    app.run(&Command::Quit);
                }
            }
            Event::IpcInput(input) => match command::parse(&input) {
                Ok(commands) => {
                    for cmd in commands {
                        info!("Executing command from IPC: {cmd}");
                        app.run(&cmd);
                    }
                }
                Err(e) => error!("Parsing error: {e}"),
            },
            Event::Input(input) => terminal.input(input, app),
            Event::Redraw => {}
        }
    }
}
