//! An audio sink for the Rodio backend (macOS's and Windows' audio) that pauses at once.
//!
//! librespot's own Rodio sink waits for its buffer (about half a second of audio) to play out
//! before it pauses, so pausing lags. This one pauses right away, keeping what's buffered to play
//! on resuming (unless it's to be dropped; see [super::discard_buffered]).

use std::thread;
use std::time::Duration;

use librespot_playback::audio_backend::{Sink, SinkBuilder, SinkError, SinkResult};
use librespot_playback::config::AudioFormat;
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use librespot_playback::{NUM_CHANNELS, SAMPLE_RATE};
use log::{info, warn};
use rodio::cpal::traits::{DeviceTrait, HostTrait};

/// Packets to keep queued: about 18 ms each, so about 110 ms in all.
const BUFFERED_PACKETS: usize = 6;

/// The [SinkBuilder] for this sink, used in place of librespot's `rodio` backend.
pub const BUILDER: SinkBuilder = open;

struct RodioSink {
    sink: rodio::Sink,
    _stream: rodio::OutputStream,
}

fn open(device: Option<String>, format: AudioFormat) -> Box<dyn Sink> {
    // Listing the devices (`-d ?`) is left to librespot's sink, which prints them and exits.
    if device.as_deref() == Some("?") {
        let librespot_rodio = librespot_playback::audio_backend::BACKENDS
            .iter()
            .find(|backend| backend.0 == "rodio")
            .expect("built with the rodio backend")
            .1;
        return librespot_rodio(device, format);
    }
    let host = rodio::cpal::default_host();
    let cpal_device = match device.as_deref() {
        Some(name) => host
            .output_devices()
            .ok()
            .and_then(|mut devices| devices.find(|d| d.name().ok().as_deref() == Some(name))),
        None => host.default_output_device(),
    }
    .expect("audio device not available");
    info!(
        "Using audio device: {}",
        cpal_device.name().unwrap_or_default()
    );
    let mut stream = rodio::OutputStreamBuilder::from_device(cpal_device)
        .and_then(|builder| builder.open_stream_or_fallback())
        .expect("couldn't open the audio device");
    stream.log_on_drop(false);
    let sink = rodio::Sink::connect_new(stream.mixer());
    Box::new(RodioSink {
        sink,
        _stream: stream,
    })
}

impl Sink for RodioSink {
    fn start(&mut self) -> SinkResult<()> {
        if super::take_discard() {
            self.sink.clear();
        }
        self.sink.play();
        Ok(())
    }

    fn stop(&mut self) -> SinkResult<()> {
        // What's buffered stays, to play on resuming, rather than being waited for.
        self.sink.pause();
        Ok(())
    }

    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        let samples = packet
            .samples()
            .map_err(|e| SinkError::OnWrite(e.to_string()))?;
        let samples: &[f32] = &converter.f64_to_f32(samples);
        self.sink.append(rodio::buffer::SamplesBuffer::new(
            u16::from(NUM_CHANNELS),
            SAMPLE_RATE,
            samples,
        ));
        while self.sink.len() > BUFFERED_PACKETS {
            if self.sink.is_paused() {
                warn!("Audio written while paused");
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
}
