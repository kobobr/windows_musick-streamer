pub const OUTPUT_SAMPLE_RATE: u32 = 48_000;
pub const OUTPUT_CHANNELS: u16 = 2;

pub fn samples_to_stereo_i16_bytes(samples: &[f32], channels: u16, sample_rate: u32) -> Vec<u8> {
    samples_to_stereo_i16_bytes_with_gain(samples, channels, sample_rate, 1.0)
}

pub fn samples_to_stereo_i16_bytes_with_gain(
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
    gain: f32,
) -> Vec<u8> {
    samples_to_stereo_i16_bytes_with_endianness(
        samples,
        channels,
        sample_rate,
        OUTPUT_SAMPLE_RATE,
        gain,
        false,
    )
}

pub fn samples_to_stereo_i16_be_bytes_with_gain(
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
    gain: f32,
) -> Vec<u8> {
    samples_to_stereo_i16_bytes_with_endianness(
        samples,
        channels,
        sample_rate,
        OUTPUT_SAMPLE_RATE,
        gain,
        true,
    )
}

pub fn samples_to_stereo_i16_bytes_at_sample_rate_with_gain(
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
    output_sample_rate: u32,
    gain: f32,
) -> Vec<u8> {
    samples_to_stereo_i16_bytes_with_endianness(
        samples,
        channels,
        sample_rate,
        output_sample_rate,
        gain,
        false,
    )
}

fn samples_to_stereo_i16_bytes_with_endianness(
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
    output_sample_rate: u32,
    gain: f32,
    big_endian: bool,
) -> Vec<u8> {
    let stereo = to_stereo(samples, channels, sample_rate, output_sample_rate);
    let mut bytes = Vec::with_capacity(stereo.len() * OUTPUT_CHANNELS as usize * 2);
    let gain = gain.clamp(0.0, 2.0);
    for [left, right] in stereo {
        push_i16(&mut bytes, left * gain, big_endian);
        push_i16(&mut bytes, right * gain, big_endian);
    }
    bytes
}

pub fn wav_stream_header() -> [u8; 44] {
    let byte_rate = OUTPUT_SAMPLE_RATE * OUTPUT_CHANNELS as u32 * 2;
    let block_align = OUTPUT_CHANNELS * 2;
    let data_size = u32::MAX - 36;
    let riff_size = u32::MAX;
    let mut header = [0u8; 44];

    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&riff_size.to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes());
    header[20..22].copy_from_slice(&1u16.to_le_bytes());
    header[22..24].copy_from_slice(&OUTPUT_CHANNELS.to_le_bytes());
    header[24..28].copy_from_slice(&OUTPUT_SAMPLE_RATE.to_le_bytes());
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    header[32..34].copy_from_slice(&block_align.to_le_bytes());
    header[34..36].copy_from_slice(&16u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data_size.to_le_bytes());

    header
}

fn to_stereo(
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
    output_sample_rate: u32,
) -> Vec<[f32; 2]> {
    let channels = channels.max(1) as usize;
    let input_frames = samples.len() / channels;
    if input_frames == 0 {
        return Vec::new();
    }

    if sample_rate == output_sample_rate {
        return samples
            .chunks(channels)
            .map(|frame| {
                let left = frame.first().copied().unwrap_or_default();
                let right = frame.get(1).copied().unwrap_or(left);
                [left, right]
            })
            .collect();
    }

    let output_frames =
        ((input_frames as u64 * output_sample_rate as u64) / sample_rate.max(1) as u64) as usize;
    let mut output = Vec::with_capacity(output_frames);
    for out_idx in 0..output_frames {
        let src_pos = out_idx as f64 * sample_rate as f64 / output_sample_rate as f64;
        let src_idx = src_pos.floor() as usize;
        let next_idx = (src_idx + 1).min(input_frames - 1);
        let frac = (src_pos - src_idx as f64) as f32;
        let frame = |idx: usize, channel: usize| -> f32 {
            samples
                .get(idx * channels + channel.min(channels - 1))
                .copied()
                .unwrap_or_default()
        };
        let left = lerp(frame(src_idx, 0), frame(next_idx, 0), frac);
        let right_source = if channels > 1 { 1 } else { 0 };
        let right = lerp(
            frame(src_idx, right_source),
            frame(next_idx, right_source),
            frac,
        );
        output.push([left, right]);
    }
    output
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn push_i16(buffer: &mut Vec<u8>, sample: f32, big_endian: bool) {
    let sample = sample.clamp(-1.0, 1.0);
    let value = (sample * i16::MAX as f32) as i16;
    if big_endian {
        buffer.extend_from_slice(&value.to_be_bytes());
    } else {
        buffer.extend_from_slice(&value.to_le_bytes());
    }
}
