//! Byte-stable for byte-stable input, so a committed narration directory
//! can be diffed.

use std::io::Read;

use super::Pcm;

const HEADER_LEN: usize = 44;
const BITS_PER_SAMPLE: u16 = 16;

/// A chunk's length as a WAVE header states it. A header cannot say more
/// than 4 GiB, so a longer one says the most it can, which readers of
/// streamed files take to mean "to the end of the file".
fn chunk_len(bytes: usize) -> u32 {
    u32::try_from(bytes).unwrap_or(u32::MAX)
}

/// Canonical WAVE: one `fmt ` chunk, one `data` chunk, no padding.
pub fn encode(pcm: &Pcm) -> Vec<u8> {
    let channels = pcm.channels;
    let block_align = channels * (BITS_PER_SAMPLE / 8);
    let byte_rate = pcm.sample_rate * block_align as u32;
    let data_len = chunk_len(pcm.samples.len() * 2);

    let mut out = Vec::with_capacity(HEADER_LEN + pcm.samples.len() * 2);

    out.extend_from_slice(b"RIFF");
    // Bytes after this field: "WAVE" 4 + fmt chunk 24 + data header 8 + data.
    out.extend_from_slice(&data_len.saturating_add(36).to_le_bytes());
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
    let mut reader = Reader::new(bytes)?;
    let mut samples = Vec::with_capacity(bytes.len() / 2);
    while let Some(block) = reader.next_block()? {
        samples.extend(block);
    }
    Ok(Pcm {
        sample_rate: reader.sample_rate,
        channels: reader.channels,
        samples,
    })
}

/// The first channel of a WAVE read from `r`, the others dropped as it is
/// read: a long stereo recording never sits in memory whole.
pub fn first_channel(r: impl Read) -> Result<Pcm, String> {
    let mut reader = Reader::new(r)?;
    let step = usize::from(reader.channels.max(1));
    let mut samples = Vec::new();
    while let Some(block) = reader.next_block()? {
        samples.extend(block.iter().step_by(step));
    }
    Ok(Pcm {
        sample_rate: reader.sample_rate,
        channels: 1,
        samples,
    })
}

const NOT_WAVE: &str = "not a WAVE file";
const PAST_END: &str = "a chunk runs past the end of the file";
const NO_DATA: &str = "no `data` chunk";

/// A 16-bit PCM WAVE read a block of whole frames at a time.
pub struct Reader<R> {
    inner: R,
    sample_rate: u32,
    channels: u16,
    /// Bytes of the `data` chunk not yet read.
    left: u64,
    /// Whether the `data` chunk's length is a placeholder, as a WAVE
    /// streamed while it was made says: the samples are what follows.
    streamed: bool,
}

/// A `data` length at least this large is a streamed WAVE's placeholder.
const STREAMED: u32 = 0x7FFF_0000;

impl<R: Read> Reader<R> {
    /// Reads the header up to the start of the samples.
    pub fn new(mut inner: R) -> Result<Self, String> {
        let mut riff = [0u8; 12];
        if fill(&mut inner, &mut riff)? < 12 || &riff[..4] != b"RIFF" || &riff[8..] != b"WAVE" {
            return Err(NOT_WAVE.to_string());
        }
        let mut format: Option<(u32, u16)> = None;
        loop {
            let mut head = [0u8; 8];
            if fill(&mut inner, &mut head)? < 8 {
                return Err(NO_DATA.to_string());
            }
            let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
            match &head[..4] {
                b"fmt " if len >= 16 => {
                    let mut body = Vec::new();
                    (&mut inner)
                        .take(u64::from(len))
                        .read_to_end(&mut body)
                        .map_err(|e| e.to_string())?;
                    if body.len() < len as usize {
                        return Err(PAST_END.to_string());
                    }
                    let u16_at = |i: usize| u16::from_le_bytes([body[i], body[i + 1]]);
                    if u16_at(0) != 1 || u16_at(14) != BITS_PER_SAMPLE {
                        return Err("only 16-bit PCM is supported".to_string());
                    }
                    let rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                    format = Some((rate, u16_at(2)));
                    skip(&mut inner, u64::from(len % 2))?;
                }
                b"data" => {
                    let (sample_rate, channels) =
                        format.ok_or("no `fmt ` chunk before the data")?;
                    return Ok(Self {
                        inner,
                        sample_rate,
                        channels,
                        left: u64::from(len),
                        streamed: len >= STREAMED,
                    });
                }
                _ => {
                    let want = u64::from(len);
                    if skip(&mut inner, want)? < want {
                        return Err(PAST_END.to_string());
                    }
                    skip(&mut inner, want % 2)?;
                }
            }
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// The next samples, interleaved, or `None` after the last.
    pub fn next_block(&mut self) -> Result<Option<Vec<i16>>, String> {
        let frame = u64::from(self.channels.max(1)) * 2;
        let want = self.left.min((BLOCK / frame).max(1) * frame);
        // An odd last byte is half a sample: dropped.
        let want = want - want % 2;
        if want == 0 {
            return Ok(None);
        }
        let mut bytes = vec![0u8; want as usize];
        let got = fill(&mut self.inner, &mut bytes)?;
        if got < bytes.len() {
            if !self.streamed {
                return Err(PAST_END.to_string());
            }
            // The end of a streamed WAVE: its last whole frames.
            bytes.truncate(got - got % frame as usize);
            self.left = 0;
            return Ok((!bytes.is_empty()).then(|| {
                bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&s| i16::from_le_bytes(s))
                    .collect()
            }));
        }
        self.left -= want;
        Ok(Some(
            bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&s| i16::from_le_bytes(s))
                .collect(),
        ))
    }
}

/// Bytes of samples read at once.
const BLOCK: u64 = 64 * 1024;

/// Reads until `buf` is full or the input ends; how much was read.
fn fill(r: &mut impl Read, buf: &mut [u8]) -> Result<usize, String> {
    let mut got = 0;
    while got < buf.len() {
        match r.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(got)
}

/// Discards up to `n` bytes; how many there were.
fn skip(r: &mut impl Read, n: u64) -> Result<u64, String> {
    std::io::copy(&mut r.take(n), &mut std::io::sink()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod chunk_tests {
    use super::chunk_len;

    #[test]
    fn a_chunk_longer_than_a_header_can_say_says_the_most_it_can() {
        assert_eq!(chunk_len(88_200), 88_200);
        assert_eq!(chunk_len(u32::MAX as usize), u32::MAX);
        #[cfg(target_pointer_width = "64")]
        assert_eq!(chunk_len(u32::MAX as usize + 2), u32::MAX);
    }
}
