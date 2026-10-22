//! An audio sink for PulseAudio (Linux's and the BSDs' audio; PipeWire provides it) that stops at
//! once.
//!
//! librespot's own PulseAudio sink drains its buffer when it stops, so pausing lags and skipping
//! plays a moment of the old song. This one flushes what's buffered instead. PulseAudio's simple
//! API can't pause a stream, so it keeps a copy of what it wrote, and resuming plays again the part
//! that wasn't heard (unless it's to be dropped; see [super::discard_buffered]).

use std::collections::VecDeque;

use libpulse_binding::def::BufferAttr;
use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::Direction;
use libpulse_binding::time::MicroSeconds;
use libpulse_simple_binding::Simple;
use librespot_playback::audio_backend::{Sink, SinkBuilder, SinkError, SinkResult};
use librespot_playback::config::AudioFormat;
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use librespot_playback::{NUM_CHANNELS, SAMPLE_RATE};
use log::info;

/// Audio to keep buffered: 100 ms.
const BUFFERED: MicroSeconds = MicroSeconds(100_000);

/// Audio written to keep a copy of, at least as much as can be unheard when stopping: 1 s.
const KEPT: MicroSeconds = MicroSeconds(1_000_000);

/// Samples are written as f32, whatever format librespot asks for.
const SPEC: Spec = Spec {
    format: Format::FLOAT32NE,
    channels: NUM_CHANNELS,
    rate: SAMPLE_RATE,
};

/// The [SinkBuilder] for this sink, used in place of librespot's `pulseaudio` backend.
pub const BUILDER: SinkBuilder = open;

struct PulseAudioSink {
    stream: Option<Simple>,
    device: Option<String>,
    /// The latest audio written, up to [KEPT]; once stopped, only what wasn't heard.
    written: VecDeque<u8>,
}

fn open(device: Option<String>, _format: AudioFormat) -> Box<dyn Sink> {
    info!("Using PulseAudio sink, device: {device:?}");
    Box::new(PulseAudioSink {
        stream: None,
        device,
        written: VecDeque::new(),
    })
}

fn write_error(e: libpulse_binding::error::PAErr) -> SinkError {
    SinkError::OnWrite(format!("{e}"))
}

impl PulseAudioSink {
    fn stream(&self) -> SinkResult<&Simple> {
        self.stream
            .as_ref()
            .ok_or_else(|| SinkError::NotConnected("PulseAudio stream not started".into()))
    }
}

impl Sink for PulseAudioSink {
    fn start(&mut self) -> SinkResult<()> {
        if self.stream.is_some() {
            return Ok(());
        }
        let buffer = BufferAttr {
            maxlength: u32::MAX,
            tlength: SPEC.usec_to_bytes(BUFFERED) as u32,
            prebuf: u32::MAX,
            minreq: u32::MAX,
            fragsize: u32::MAX,
        };
        // The other stream properties (media.role) come from the environment; see
        // crate::spotify::pulseaudio_environment.
        let stream = Simple::new(
            None,
            "ttunes",
            Direction::Playback,
            self.device.as_deref(),
            "Spotify player for the terminal",
            &SPEC,
            None,
            Some(&buffer),
        )
        .map_err(|e| SinkError::ConnectionRefused(format!("{e}")))?;
        if super::take_discard() {
            self.written.clear();
        }
        if !self.written.is_empty() {
            stream
                .write(self.written.make_contiguous())
                .map_err(write_error)?;
        }
        self.stream = Some(stream);
        Ok(())
    }

    fn stop(&mut self) -> SinkResult<()> {
        let Some(stream) = self.stream.take() else {
            return Ok(());
        };
        // What's buffered is dropped rather than played out. What wasn't heard of it is how much
        // less there is to play once it's dropped (the device still plays what it has).
        let before = stream.get_latency().unwrap_or(MicroSeconds(0));
        stream.flush().map_err(write_error)?;
        let after = stream.get_latency().unwrap_or(before);
        let unheard = SPEC
            .usec_to_bytes(MicroSeconds(before.0.saturating_sub(after.0)))
            .min(self.written.len());
        self.written.drain(..self.written.len() - unheard);
        Ok(())
    }

    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        let samples = packet
            .samples()
            .map_err(|e| SinkError::OnWrite(e.to_string()))?;
        let samples = converter.f64_to_f32(samples);
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_ne_bytes()).collect();
        self.stream()?.write(&bytes).map_err(write_error)?;
        self.written.extend(bytes);
        let kept = SPEC.usec_to_bytes(KEPT);
        if self.written.len() > kept {
            self.written.drain(..self.written.len() - kept);
        }
        Ok(())
    }
}
