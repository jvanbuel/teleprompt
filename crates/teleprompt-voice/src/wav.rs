//! Byte-stable for byte-stable input, so a committed narration directory
//! can be diffed.

use crate::contract::Pcm;

const HEADER_LEN: usize = 44;
const BITS_PER_SAMPLE: u16 = 16;

/// Canonical WAVE: one `fmt ` chunk, one `data` chunk, no padding.
pub fn encode(pcm: &Pcm) -> Vec<u8> {
    let channels = pcm.channels;
    let block_align = channels * (BITS_PER_SAMPLE / 8);
    let byte_rate = pcm.sample_rate * block_align as u32;
    let data_len = (pcm.samples.len() * 2) as u32;

    let mut out = Vec::with_capacity(HEADER_LEN + data_len as usize);

    out.extend_from_slice(b"RIFF");
    // Bytes after this field: "WAVE" 4 + fmt chunk 24 + data header 8 + data.
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

/// 16-bit PCM WAVE, as [`encode`] writes it; other chunks are skipped.
pub fn decode(bytes: &[u8]) -> Result<Pcm, String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAVE file".to_string());
    }
    let u16_at = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let u32_at = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let mut format: Option<(u32, u16)> = None;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let len = u32_at(bytes, at + 4) as usize;
        let body = bytes
            .get(at + 8..at + 8 + len)
            .ok_or("a chunk runs past the end of the file")?;
        match &bytes[at..at + 4] {
            b"fmt " if len >= 16 => {
                if u16_at(body, 0) != 1 || u16_at(body, 14) != BITS_PER_SAMPLE {
                    return Err("only 16-bit PCM is supported".to_string());
                }
                format = Some((u32_at(body, 4), u16_at(body, 2)));
            }
            b"data" => {
                let (sample_rate, channels) = format.ok_or("no `fmt ` chunk before the data")?;
                return Ok(Pcm {
                    sample_rate,
                    channels,
                    samples: body
                        .chunks_exact(2)
                        .map(|s| i16::from_le_bytes([s[0], s[1]]))
                        .collect(),
                });
            }
            _ => {}
        }
        at += 8 + len + len % 2;
    }
    Err("no `data` chunk".to_string())
}
