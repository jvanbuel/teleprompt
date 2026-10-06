//! Hearing a recording of a conversation: its words, when each was said,
//! and who said them. Behind the `listen` feature, like everything that
//! runs a speech model.

use teleprompt_listen::SpeakerSpan;
use teleprompt_voice::Pcm;

#[cfg(feature = "listen")]
use teleprompt_setup as setup;

/// A word heard, and when: its text, start and end, in milliseconds.
pub type Heard = (String, u64, u64);

/// What a conversation's recording says, and who speaks when: `None` for
/// the voices when the speaker models are not installed.
pub struct Conversation {
    pub words: Vec<Heard>,
    pub voices: Option<Vec<SpeakerSpan>>,
}

/// Hears `pcm` with the models `setup` installed: the speech model, which
/// must be there; the punctuation model, for capitals and full stops,
/// where it is; and the speaker models, where they are. `speakers` is how
/// many voices there are, where the caller knows.
#[cfg(feature = "listen")]
pub fn hear(pcm: &Pcm, speakers: Option<usize>) -> Result<Conversation, String> {
    let samples = at_16k(pcm);
    let model = setup::speech_model(None)?;
    let words = teleprompt_listen::sherpa::transcribe(&model, &samples)?;
    let words: Vec<teleprompt_derive::Word> = words
        .into_iter()
        .map(|w| teleprompt_derive::Word {
            text: w.text,
            start_ms: w.start_ms,
            end_ms: w.end_ms,
        })
        .collect();
    let punctuation = setup::punctuation_model(None).or_else(|| {
        teleprompt_setup::offer(
            &["punctuation-model"],
            "give the draft capitals and full stops",
        )
        .then(|| setup::punctuation_model(None))
        .flatten()
    });
    let words = match punctuation {
        Some(dir) => {
            let text: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
            let text = teleprompt_listen::sherpa::punctuate(&dir, &text.join(" "))?;
            teleprompt_derive::punctuate(&words, &text)
        }
        // The model hears in capitals; without punctuation, lower case
        // reads better than shouting.
        None => words
            .into_iter()
            .map(|w| teleprompt_derive::Word {
                text: w.text.to_lowercase(),
                ..w
            })
            .collect(),
    };
    let speaker_models = setup::speaker_models().or_else(|| {
        teleprompt_setup::offer(&["speaker-model"], "tell the voices apart")
            .then(setup::speaker_models)
            .flatten()
    });
    let voices = match speaker_models {
        Some(dir) => Some(teleprompt_listen::sherpa::diarize(
            &dir, &samples, speakers,
        )?),
        None => None,
    };
    Ok(Conversation {
        words: words
            .into_iter()
            .map(|w| (w.text, w.start_ms, w.end_ms))
            .collect(),
        voices,
    })
}

#[cfg(not(feature = "listen"))]
pub fn hear(_: &Pcm, _: Option<usize>) -> Result<Conversation, String> {
    Err(
        "this teleprompt was built without speech models, so it cannot transcribe a \
         recording: rebuild it with `--features listen`, or draft from a transcript"
            .to_string(),
    )
}

/// `pcm` as the speech models take it: floats at 16 kHz.
#[cfg(feature = "listen")]
pub fn at_16k(pcm: &Pcm) -> Vec<f32> {
    let mut resampler =
        teleprompt_voice::Resampler::new(pcm.sample_rate, teleprompt_listen::sherpa::SAMPLE_RATE);
    let mut samples = Vec::new();
    for block in pcm.samples.chunks(1 << 16) {
        let block: Vec<f32> = block.iter().map(|&s| f32::from(s) / 32768.0).collect();
        samples.extend(resampler.push(&block));
    }
    samples.extend(resampler.finish());
    samples
}
