//! A 16-bit PCM RIFF/WAVE encoder, which is 30 lines and therefore not
//! worth a dependency. Output is byte-stable for byte-stable input, which
//! is what lets a committed narration directory be diffed.

use crate::contract::Pcm;

const HEADER_LEN: usize = 44;
const BITS_PER_SAMPLE: u16 = 16;

/// Encode `pcm` as a canonical WAVE file: one `fmt ` chunk, one `data`
/// chunk, no padding, little-endian throughout.
pub fn encode(pcm: &Pcm) -> Vec<u8> {
    let channels = pcm.channels;
    let block_align = channels * (BITS_PER_SAMPLE / 8);
    let byte_rate = pcm.sample_rate * block_align as u32;
    let data_len = (pcm.samples.len() * 2) as u32;

    let mut out = Vec::with_capacity(HEADER_LEN + data_len as usize);

    out.extend_from_slice(b"RIFF");
    // Everything after this field: 4 ("WAVE") + 24 (fmt chunk) + 8 (data
    // header) + the samples.
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");

    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // WAVE_FORMAT_PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&pcm.sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());

    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in &pcm.samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }

    out
}
