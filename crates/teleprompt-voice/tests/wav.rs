use teleprompt_voice::{wav, Pcm};

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
