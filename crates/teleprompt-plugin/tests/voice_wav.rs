use teleprompt_plugin::voice::{wav, Pcm};

/// Three mono samples at 48 kHz. Every header field is hand-computed here
/// rather than read back through our own decoder, because a decoder that
/// shares the encoder's misunderstanding agrees with it perfectly.
#[test]
fn encodes_a_canonical_44_byte_header() {
    let pcm = Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0, 1, -1],
    };
    let out = wav::encode(&pcm);

    assert_eq!(
        out.len(),
        44 + 6,
        "44-byte header plus 3 samples of 2 bytes"
    );

    assert_eq!(&out[0..4], b"RIFF");
    assert_eq!(&out[4..8], &42u32.to_le_bytes(), "36 + data length");
    assert_eq!(&out[8..12], b"WAVE");

    assert_eq!(&out[12..16], b"fmt ");
    assert_eq!(
        &out[16..20],
        &16u32.to_le_bytes(),
        "PCM fmt chunk is 16 bytes"
    );
    assert_eq!(&out[20..22], &1u16.to_le_bytes(), "format tag 1 = PCM");
    assert_eq!(&out[22..24], &1u16.to_le_bytes(), "channels");
    assert_eq!(&out[24..28], &48_000u32.to_le_bytes(), "sample rate");
    assert_eq!(
        &out[28..32],
        &96_000u32.to_le_bytes(),
        "byte rate = rate * block align"
    );
    assert_eq!(
        &out[32..34],
        &2u16.to_le_bytes(),
        "block align = channels * 2"
    );
    assert_eq!(&out[34..36], &16u16.to_le_bytes(), "bits per sample");

    assert_eq!(&out[36..40], b"data");
    assert_eq!(&out[40..44], &6u32.to_le_bytes(), "data length");

    assert_eq!(&out[44..46], &0i16.to_le_bytes());
    assert_eq!(&out[46..48], &1i16.to_le_bytes());
    assert_eq!(&out[48..50], &(-1i16).to_le_bytes());
}

#[test]
fn encodes_stereo_block_alignment() {
    let pcm = Pcm {
        sample_rate: 44_100,
        channels: 2,
        samples: vec![0; 4],
    };
    let out = wav::encode(&pcm);
    assert_eq!(&out[22..24], &2u16.to_le_bytes(), "channels");
    assert_eq!(
        &out[32..34],
        &4u16.to_le_bytes(),
        "block align = 2 channels * 2 bytes"
    );
    assert_eq!(&out[28..32], &176_400u32.to_le_bytes(), "44100 * 4");
}

#[test]
fn an_empty_pcm_is_a_valid_header_with_no_data() {
    let pcm = Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![],
    };
    let out = wav::encode(&pcm);
    assert_eq!(out.len(), 44);
    assert_eq!(&out[40..44], &0u32.to_le_bytes());
    assert_eq!(&out[4..8], &36u32.to_le_bytes());
}

#[test]
fn encoding_is_byte_stable() {
    let pcm = Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0; 480],
    };
    assert_eq!(wav::encode(&pcm), wav::encode(&pcm));
}

/// A take is read back from the WAV it was saved as.
#[test]
fn decoding_what_was_encoded_gives_it_back() {
    let pcm = teleprompt_plugin::voice::Pcm {
        sample_rate: 24_000,
        channels: 1,
        samples: vec![0, 1, -1, i16::MAX, i16::MIN, 1234],
    };
    let wav = teleprompt_plugin::voice::wav::encode(&pcm);
    assert_eq!(teleprompt_plugin::voice::wav::decode(&wav).unwrap(), pcm);
    assert!(teleprompt_plugin::voice::wav::decode(b"RIFF....WAVEnot a wav").is_err());
}

/// A stereo recording with a `LIST` chunk before its data, as recorders
/// write them.
fn recorded(frames: usize) -> Vec<u8> {
    let samples: Vec<i16> = (0..frames * 2)
        .map(|i| if i % 2 == 0 { i as i16 } else { -1 })
        .collect();
    let plain = wav::encode(&Pcm {
        sample_rate: 48_000,
        channels: 2,
        samples,
    });
    let mut out = plain[..36].to_vec();
    out.extend_from_slice(b"LIST");
    out.extend_from_slice(&3u32.to_le_bytes());
    out.extend_from_slice(b"abc\0");
    out.extend_from_slice(&plain[36..]);
    out
}

#[test]
fn a_reader_reads_in_blocks_what_decode_reads_whole() {
    let bytes = recorded(100_000);
    let mut reader = wav::Reader::new(&bytes[..]).unwrap();
    assert_eq!((reader.sample_rate(), reader.channels()), (48_000, 2));
    let mut samples = Vec::new();
    let mut blocks = 0;
    while let Some(block) = reader.next_block().unwrap() {
        assert_eq!(block.len() % 2, 0, "a block is whole frames");
        samples.extend(block);
        blocks += 1;
    }
    assert!(blocks > 1, "read in more than one block");
    assert_eq!(samples, wav::decode(&bytes).unwrap().samples);
}

#[test]
fn the_first_channel_is_read_without_the_others() {
    let pcm = wav::first_channel(&recorded(5)[..]).unwrap();
    assert_eq!((pcm.sample_rate, pcm.channels), (48_000, 1));
    assert_eq!(pcm.samples, [0, 2, 4, 6, 8]);
}

#[test]
fn a_reader_refuses_what_decode_refuses() {
    let bytes = recorded(10);
    for bad in [&bytes[..8], &bytes[..30], &bytes[..bytes.len() - 3]] {
        let whole = wav::decode(bad).unwrap_err();
        let read = wav::first_channel(bad).unwrap_err();
        assert_eq!(read, whole);
    }
}
