//! The microphone, through GStreamer, as it is: what it hears is the take.

use std::sync::{Arc, Mutex};

use gst::prelude::*;

/// The rate the microphone is captured at; the server converts it for the
/// recognizer and keeps it for the take.
pub const RATE: u32 = 48_000;

/// Captures mono samples at [`RATE`], handing them on about every 100 ms
/// while sending is on.
pub struct Mic {
    pipeline: gst::Pipeline,
    shared: Arc<Mutex<Shared>>,
}

struct Shared {
    sending: bool,
    pending: Vec<f32>,
}

impl Mic {
    /// The default microphone, or the GStreamer source in `TELEPROMPT_MIC`,
    /// such as `filesrc location=reading.wav ! wavparse`, to read to it from
    /// a recording. `sink` is called from GStreamer's thread.
    pub fn open(sink: impl Fn(Vec<f32>) + Send + Sync + 'static) -> Result<Self, String> {
        gst::init().map_err(|e| e.to_string())?;
        let source = std::env::var("TELEPROMPT_MIC").unwrap_or_else(|_| "autoaudiosrc".into());
        let description = format!(
            "{source} ! audioconvert ! audioresample ! \
             audio/x-raw,format=F32LE,channels=1,rate={RATE},layout=interleaved ! \
             appsink name=sink sync=true"
        );
        let pipeline = gst::parse::launch(&description)
            .map_err(|e| format!("the microphone pipeline did not build: {e}"))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "the microphone pipeline is not a pipeline".to_string())?;
        let appsink = pipeline
            .by_name("sink")
            .and_then(|e| e.downcast::<gst_app::AppSink>().ok())
            .ok_or("the microphone pipeline has no sink")?;
        let shared = Arc::new(Mutex::new(Shared {
            sending: false,
            pending: Vec::new(),
        }));
        let chunk = RATE as usize / 10;
        let taken = shared.clone();
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    let samples = map
                        .as_slice()
                        .chunks_exact(4)
                        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                    let ready = {
                        let mut shared = taken.lock().unwrap_or_else(|p| p.into_inner());
                        if !shared.sending {
                            return Ok(gst::FlowSuccess::Ok);
                        }
                        shared.pending.extend(samples);
                        (shared.pending.len() >= chunk).then(|| std::mem::take(&mut shared.pending))
                    };
                    if let Some(ready) = ready {
                        sink(ready);
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("the microphone did not start: {e}"))?;
        Ok(Self { pipeline, shared })
    }

    /// Whether what the microphone hears is sent; while not, it is dropped.
    pub fn set_sending(&self, sending: bool) {
        self.lock().sending = sending;
    }

    /// The samples not yet handed on.
    pub fn flush(&self) -> Vec<f32> {
        std::mem::take(&mut self.lock().pending)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl Drop for Mic {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
