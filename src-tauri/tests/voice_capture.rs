#![cfg(target_os = "macos")]

use std::ffi::c_void;

use saturday::voice::contract::MAX_CAPTURE_SAMPLES;

const OUTPUT_CAPACITY: usize = 8192;

unsafe extern "C" {
    fn saturday_audio_converter_create(
        sample_rate: u32,
        channels: u32,
        max_input_frames: u32,
        out_converter: *mut *mut c_void,
    ) -> i32;
    fn saturday_audio_converter_convert(
        converter: *mut c_void,
        input: *const f32,
        input_frames: u32,
        output: *mut f32,
        output_capacity: u32,
        output_frames: *mut u32,
    ) -> i32;
    fn saturday_audio_converter_finish(
        converter: *mut c_void,
        output: *mut f32,
        output_capacity: u32,
        output_frames: *mut u32,
    ) -> i32;
    fn saturday_audio_converter_destroy(converter: *mut c_void);
}

struct Converter(*mut c_void);

impl Converter {
    fn new(sample_rate: u32, channels: u32, max_input_frames: u32) -> Self {
        let mut converter = std::ptr::null_mut();
        let status = unsafe {
            saturday_audio_converter_create(sample_rate, channels, max_input_frames, &mut converter)
        };
        assert_eq!(status, 0, "valid native converter creation failed");
        assert!(!converter.is_null());
        Self(converter)
    }

    fn push(&mut self, interleaved: &[f32], channels: usize, output: &mut Vec<f32>) {
        assert_eq!(interleaved.len() % channels, 0);
        let mut block = [0.0; OUTPUT_CAPACITY];
        let mut frames = 0;
        let status = unsafe {
            saturday_audio_converter_convert(
                self.0,
                interleaved.as_ptr(),
                (interleaved.len() / channels) as u32,
                block.as_mut_ptr(),
                OUTPUT_CAPACITY as u32,
                &mut frames,
            )
        };
        assert_eq!(status, 0, "valid converter block rejected");
        output.extend_from_slice(&block[..frames as usize]);
    }

    fn finish(&mut self, output: &mut Vec<f32>) {
        loop {
            let mut block = [0.0; OUTPUT_CAPACITY];
            let mut frames = 0;
            let status = unsafe {
                saturday_audio_converter_finish(
                    self.0,
                    block.as_mut_ptr(),
                    OUTPUT_CAPACITY as u32,
                    &mut frames,
                )
            };
            assert_eq!(status, 0, "converter flush failed");
            output.extend_from_slice(&block[..frames as usize]);
            if frames == 0 {
                break;
            }
        }
    }
}

impl Drop for Converter {
    fn drop(&mut self) {
        unsafe { saturday_audio_converter_destroy(self.0) };
    }
}

fn resample_sine(sample_rate: u32, frequency: f32, channels: usize) -> Vec<f32> {
    let frames = sample_rate as usize;
    let input: Vec<f32> = (0..frames)
        .flat_map(|frame| {
            let sample =
                (std::f32::consts::TAU * frequency * frame as f32 / sample_rate as f32).sin() * 0.5;
            std::iter::repeat_n(sample, channels)
        })
        .collect();
    let mut converter = Converter::new(sample_rate, channels as u32, 320);
    let mut output = Vec::new();
    let mut offset = 0;
    while offset < frames {
        let count = (frames - offset).min(320);
        let start = offset * channels;
        converter.push(
            &input[start..start + count * channels],
            channels,
            &mut output,
        );
        offset += count;
    }
    converter.finish(&mut output);
    output
}

fn rms_at_frequency(samples: &[f32], frequency: f32) -> f32 {
    let (mut real, mut imaginary) = (0.0f64, 0.0f64);
    for (index, sample) in samples.iter().enumerate().skip(256) {
        let phase = std::f64::consts::TAU * frequency as f64 * index as f64 / 16_000.0;
        real += *sample as f64 * phase.cos();
        imaginary += *sample as f64 * phase.sin();
    }
    let count = samples.len().saturating_sub(256).max(1) as f64;
    (2.0 * real.hypot(imaginary) / count / 2.0f64.sqrt()) as f32
}

#[test]
fn converts_44100_and_48000_hz_to_duration_preserving_16khz_audio() {
    for rate in [44_100, 48_000] {
        let samples = resample_sine(rate, 1_000.0, 1);
        assert!(
            (samples.len() as isize - 16_000).abs() <= 2,
            "rate {rate} produced {} frames",
            samples.len()
        );
        assert!(rms_at_frequency(&samples, 1_000.0) > 0.25);
        assert!(samples
            .iter()
            .all(|sample| sample.is_finite() && (-1.0..=1.0).contains(sample)));
    }
}

#[test]
fn converter_attenuates_content_above_8khz_before_16khz_decimation() {
    let passband = resample_sine(48_000, 1_000.0, 1);
    let stopband = resample_sine(48_000, 12_000.0, 1);
    assert!(rms_at_frequency(&passband, 1_000.0) > 0.25);
    assert!(rms_at_frequency(&stopband, 4_000.0) < rms_at_frequency(&passband, 1_000.0) * 0.05);
}

#[test]
fn stereo_downmix_preserves_both_input_channels() {
    let rate = 48_000usize;
    let input: Vec<f32> = (0..rate)
        .flat_map(|frame| {
            let left = (std::f32::consts::TAU * 440.0 * frame as f32 / rate as f32).sin() * 0.5;
            let right = (std::f32::consts::TAU * 880.0 * frame as f32 / rate as f32).sin() * 0.5;
            [left, right]
        })
        .collect();
    let mut converter = Converter::new(rate as u32, 2, 320);
    let mut output = Vec::new();
    let (blocks, remainder) = input.as_chunks::<640>();
    assert!(remainder.is_empty());
    for block in blocks {
        converter.push(block, 2, &mut output);
    }
    converter.finish(&mut output);
    assert!((output.len() as isize - 16_000).abs() <= 2);
    assert!(rms_at_frequency(&output, 440.0) > 0.1);
    assert!(rms_at_frequency(&output, 880.0) > 0.1);
}

#[test]
fn converter_rejects_nonfinite_and_out_of_range_pcm() {
    let converter = Converter::new(48_000, 1, 320);
    for bad in [f32::NAN, f32::INFINITY, -1.01, 1.01] {
        let mut output = [0.0; OUTPUT_CAPACITY];
        let mut frames = 0;
        let status = unsafe {
            saturday_audio_converter_convert(
                converter.0,
                &[bad; 8] as *const [f32; 8] as *const f32,
                8,
                output.as_mut_ptr(),
                OUTPUT_CAPACITY as u32,
                &mut frames,
            )
        };
        assert_ne!(status, 0, "accepted invalid sample {bad}");
    }
}

#[test]
fn capture_budget_is_exactly_thirty_seconds_at_sixteen_kilohertz() {
    assert_eq!(MAX_CAPTURE_SAMPLES, 480_000);
    assert_eq!(
        MAX_CAPTURE_SAMPLES as usize * std::mem::size_of::<f32>(),
        1_920_000
    );
}

#[test]
fn converter_rejects_unsupported_device_formats_and_callback_quanta() {
    for (sample_rate, channels, max_frames) in [
        (7_999, 1, 320),
        (192_001, 1, 320),
        (48_000, 0, 320),
        (48_000, 3, 320),
        (48_000, 1, 0),
        (48_000, 1, 4_097),
    ] {
        let mut converter = std::ptr::null_mut();
        let status = unsafe {
            saturday_audio_converter_create(sample_rate, channels, max_frames, &mut converter)
        };
        assert_ne!(
            status, 0,
            "accepted {sample_rate}Hz/{channels}ch/{max_frames} frames"
        );
        assert!(converter.is_null());
    }
}
