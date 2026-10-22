//! ttunes' own audio sinks, used in place of librespot's: they stop at once, where librespot's
//! wait for what they have buffered to play out (so pausing lags and skipping plays a moment of
//! the old song). What's buffered is kept to play on resuming, unless [discard_buffered] says it's
//! of a track that won't go on.

use std::sync::atomic::{AtomicBool, Ordering};

use librespot_playback::audio_backend::SinkBuilder;

#[cfg(all(unix, not(target_os = "macos")))]
mod pulseaudio;
#[cfg(any(feature = "rodio_backend", target_os = "macos", windows))]
mod rodio_sink;

static DISCARD: AtomicBool = AtomicBool::new(false);

/// ttunes' sink for librespot's backend `name`, if it has one.
pub fn builder(name: &str) -> Option<SinkBuilder> {
    match name {
        #[cfg(all(unix, not(target_os = "macos")))]
        "pulseaudio" => Some(pulseaudio::BUILDER),
        #[cfg(any(feature = "rodio_backend", target_os = "macos", windows))]
        "rodio" => Some(rodio_sink::BUILDER),
        _ => None,
    }
}

/// Have the sink drop what it kept when it stopped, when it next starts, rather than play it:
/// it's of a track skipped, stopped or seeked away from.
pub fn discard_buffered() {
    DISCARD.store(true, Ordering::Relaxed);
}

/// Whether the sink, starting, should drop what it kept (see [discard_buffered]).
#[allow(dead_code)] // unused in builds with neither sink
fn take_discard() -> bool {
    DISCARD.swap(false, Ordering::Relaxed)
}
