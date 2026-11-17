//! Silero voice activity detection, used to keep audio without speech away
//! from the transcription models, which tend to invent text for it.
//!
//! The detector is optional, and Hex never downloads it: it runs only once
//! the person supplies its file (`hex model import --speech-detector`).
//! Until then every caller falls back to its level-only silence check.

use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use color_eyre::eyre::Result;
use transcribe_cpp::{Backend, Model, ModelOptions, VadOptions, VadSegment, VadSession};

use crate::transcription::models as transcription_models;

/// Speech shorter than this is ignored. Silero's default of 250 ms drops a
/// short "Ok" or "Ciao"; 160 ms keeps those and still rejects clicks, keys
/// and room noise.
const MIN_SPEECH_MS: i32 = 160;

/// Speech probability that counts as speech. Silero's default of 0.5 misses
/// a whisper at -43 dBFS, which the level gate deliberately keeps; 0.4 hears
/// it and still rejects the silence in a recorded meeting.
pub(crate) const THRESHOLD: f64 = 0.4;

#[cfg_attr(test, allow(dead_code))]
enum State {
    Unloaded,
    Loaded(VadSession),
    Failed,
}

#[cfg_attr(test, allow(dead_code))]
static STATE: Mutex<State> = Mutex::new(State::Unloaded);
#[cfg_attr(test, allow(dead_code))]
static MISSING_LOGGED: AtomicBool = AtomicBool::new(false);
#[cfg_attr(test, allow(dead_code))]
static RUN_FAILURE_LOGGED: AtomicBool = AtomicBool::new(false);

/// Whether 16 kHz mono `samples` contain speech, or `None` when the detector
/// is not available and the caller should keep its current behavior.
pub(crate) fn contains_speech(samples: &[f32]) -> Option<bool> {
    #[cfg(test)]
    {
        let _ = samples;
        OVERRIDE.with(|verdict| verdict.get())
    }
    #[cfg(not(test))]
    {
        heard(samples).map(|segments| !segments.is_empty())
    }
}

/// First and last speech sample of 16 kHz mono `samples`, or `None` when the
/// detector is not available or hears no speech. The bounds are Silero's own
/// segments, which already carry its `speech_pad_ms`.
pub(crate) fn speech_bounds(samples: &[f32]) -> Option<Range<usize>> {
    #[cfg(test)]
    {
        let _ = samples;
        None
    }
    #[cfg(not(test))]
    {
        let segments = heard(samples)?;
        bounds(
            segments
                .iter()
                .map(|segment| (segment.start_sample, segment.end_sample)),
            samples.len(),
        )
    }
}

/// The span of `segments`, each a first speech sample and the first sample
/// after it, in a clip of `len` samples; clamped to the clip.
fn bounds(segments: impl Iterator<Item = (i64, i64)>, len: usize) -> Option<Range<usize>> {
    let (mut first, mut last) = (i64::MAX, i64::MIN);
    let mut heard = false;
    for (start, end) in segments {
        heard = true;
        first = first.min(start);
        last = last.max(end);
    }
    if !heard {
        return None;
    }
    let start = usize::try_from(first.max(0)).unwrap_or(usize::MAX).min(len);
    let end = usize::try_from(last.max(0)).unwrap_or(usize::MAX).min(len);
    (start < end).then_some(start..end)
}

/// `bounds` widened by `margin` samples on each side and clamped to a clip of
/// `len` samples.
pub(crate) fn edge_bounds(len: usize, bounds: Range<usize>, margin: usize) -> Range<usize> {
    let start = bounds.start.min(len).saturating_sub(margin);
    let end = bounds.end.min(len).saturating_add(margin).min(len);
    start..end.max(start)
}

/// The detector's segments, or `None` when it is unavailable or failed.
#[cfg(not(test))]
fn heard(samples: &[f32]) -> Option<Vec<VadSegment>> {
    let mut state = lock_state();
    let session = loaded_session(&mut state)?;
    match segments(session, samples) {
        Ok(segments) => Some(segments),
        Err(error) => {
            if !RUN_FAILURE_LOGGED.swap(true, Ordering::Relaxed) {
                tracing::warn!(%error, "speech detection failed");
            }
            None
        }
    }
}

/// Loads an installed detector in the background so the first dictation does
/// not pay for it. It never downloads.
pub(crate) fn prewarm() {
    #[cfg(not(test))]
    if transcription_models::speech_detector_installed() {
        let _ = std::thread::Builder::new()
            .name("hex-speech-detector".into())
            .spawn(|| {
                let _ = loaded_session(&mut lock_state());
            });
    }
}

#[cfg_attr(test, allow(dead_code))]
fn lock_state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|error| error.into_inner())
}

/// The loaded session, loading it on first use. A missing detector stays
/// unloaded, so it loads once the person supplies the file.
#[cfg_attr(test, allow(dead_code))]
fn loaded_session(state: &mut State) -> Option<&mut VadSession> {
    if matches!(state, State::Unloaded) {
        let Some(path) = transcription_models::installed_speech_detector() else {
            if !MISSING_LOGGED.swap(true, Ordering::Relaxed) {
                tracing::info!("no speech detector supplied; using the level-only silence check");
            }
            return None;
        };
        *state = match load(&path) {
            Ok(session) => State::Loaded(session),
            Err(error) => {
                tracing::warn!(%error, "could not load the speech detector");
                State::Failed
            }
        };
    }
    match state {
        State::Loaded(session) => Some(session),
        State::Unloaded | State::Failed => None,
    }
}

/// Loads the detector on the CPU: it loads in tens of milliseconds there,
/// while a first Metal start can take seconds and would hold up dictation.
fn load(path: &Path) -> Result<VadSession> {
    // Its load would otherwise register every backend unguarded (#662).
    #[cfg(target_os = "windows")]
    crate::transcription::local_transcriber::init_backends()?;
    let model = Model::load_with(
        path,
        &ModelOptions {
            backend: Backend::Cpu,
            ..Default::default()
        },
    )?;
    Ok(model.vad_session()?)
}

/// Whether one run hears speech, for the ignored tests that load the real
/// detector.
#[cfg(test)]
fn detect(session: &mut VadSession, samples: &[f32]) -> Result<bool> {
    Ok(!segments(session, samples)?.is_empty())
}

fn segments(session: &mut VadSession, samples: &[f32]) -> Result<Vec<VadSegment>> {
    let options = VadOptions {
        threshold: THRESHOLD,
        min_speech_ms: MIN_SPEECH_MS,
        ..VadOptions::default()
    };
    Ok(session.run(samples, &options)?.segments)
}

#[cfg(test)]
thread_local! {
    static OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Makes `contains_speech` answer `verdict` on this thread until the guard
/// drops. Tests never load the real detector.
#[cfg(test)]
pub(crate) fn override_for_test(verdict: Option<bool>) -> OverrideGuard {
    OverrideGuard(OVERRIDE.with(|current| current.replace(verdict)))
}

#[cfg(test)]
pub(crate) struct OverrideGuard(Option<bool>);

#[cfg(test)]
impl Drop for OverrideGuard {
    fn drop(&mut self) {
        OVERRIDE.with(|current| current.set(self.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_decides_and_restores() {
        {
            let _guard = override_for_test(Some(false));
            assert_eq!(contains_speech(&[0.0; 16_000]), Some(false));
        }
        assert_eq!(contains_speech(&[0.0; 16_000]), None);
    }

    #[test]
    fn detected_segments_bound_the_speech_inside_the_clip() {
        assert_eq!(
            bounds([(1_000, 4_000), (9_000, 12_000)].into_iter(), 16_000),
            Some(1_000..12_000)
        );
        assert_eq!(
            bounds([(12_000, 16_000)].into_iter(), 16_000),
            Some(12_000..16_000)
        );
        assert_eq!(
            bounds([(12_000, 20_000)].into_iter(), 16_000),
            Some(12_000..16_000),
            "a segment past the clip is clamped to it"
        );
        assert_eq!(bounds([].into_iter(), 16_000), None);
        assert_eq!(
            bounds([(16_000, 18_000)].into_iter(), 16_000),
            None,
            "speech past the clip leaves nothing to keep"
        );
    }

    #[test]
    fn a_margin_around_the_speech_clamps_to_the_clip() {
        // 1 s of speech inside 3 s, with the 500 ms margin a Whisper run uses.
        assert_eq!(edge_bounds(48_000, 16_000..32_000, 8_000), 8_000..40_000);
        assert_eq!(
            edge_bounds(48_000, 2_000..4_000, 8_000),
            0..12_000,
            "the leading margin stops at the clip's start"
        );
        assert_eq!(
            edge_bounds(48_000, 44_000..47_000, 8_000),
            36_000..48_000,
            "the trailing margin stops at the clip's end"
        );
        assert_eq!(
            edge_bounds(48_000, 40_000..80_000, 8_000),
            32_000..48_000,
            "bounds past the end still clamp"
        );
    }

    #[test]
    fn a_corrupt_detector_file_is_not_loaded() {
        let path = std::env::temp_dir().join(format!(
            "hex-corrupt-speech-detector-{}.gguf",
            std::process::id()
        ));
        std::fs::write(&path, b"not a gguf file").unwrap();
        let loaded = load(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(loaded.is_err());
    }

    /// First use without a supplied detector: nothing loads, nothing is
    /// fetched, and the models folder stays as it was.
    #[test]
    fn a_missing_detector_stays_unloaded_and_is_never_downloaded() {
        let models =
            std::env::temp_dir().join(format!("hex-no-speech-detector-{}", std::process::id()));
        std::fs::create_dir_all(&models).unwrap();
        transcription_models::set_test_models_dir(Some(models.clone()));
        let attempts = crate::net::attempts_for_test();
        let mut state = State::Unloaded;
        assert!(loaded_session(&mut state).is_none());
        assert!(loaded_session(&mut state).is_none());
        assert!(matches!(state, State::Unloaded));
        let left = std::fs::read_dir(&models).unwrap().count();
        transcription_models::set_test_models_dir(None);
        std::fs::remove_dir_all(&models).unwrap();
        assert_eq!(left, 0, "nothing was written to the models folder");
        assert_eq!(
            crate::net::attempts_for_test(),
            attempts,
            "no download was attempted"
        );
    }

    #[test]
    #[ignore = "requires HEX_VAD_MODEL (Silero VAD GGUF)"]
    fn real_detector_rejects_silence_and_noise() {
        let mut session = load(Path::new(&std::env::var("HEX_VAD_MODEL").unwrap())).unwrap();
        assert!(!detect(&mut session, &[0.0; 32_000]).unwrap());
        // -60 dBFS white noise from a fixed linear congruential generator.
        let mut seed = 0x2545_f491_u32;
        let noise: Vec<f32> = (0..32_000)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed as f32 / u32::MAX as f32 * 2.0 - 1.0) * 0.001 * 3f32.sqrt()
            })
            .collect();
        assert!(!detect(&mut session, &noise).unwrap());
    }

    fn clip(variable: &str) -> Vec<f32> {
        let mut reader = hound::WavReader::open(std::env::var(variable).unwrap()).unwrap();
        assert_eq!(reader.spec().sample_rate, 16_000);
        reader.samples::<f32>().map(Result::unwrap).collect()
    }

    #[test]
    #[ignore = "requires HEX_VAD_MODEL and HEX_VAD_SPEECH_WAV (16 kHz mono float speech)"]
    fn real_detector_hears_speech() {
        let mut session = load(Path::new(&std::env::var("HEX_VAD_MODEL").unwrap())).unwrap();
        assert!(detect(&mut session, &clip("HEX_VAD_SPEECH_WAV")).unwrap());
    }

    #[test]
    #[ignore = "requires HEX_VAD_MODEL and HEX_VAD_WHISPER_WAV (16 kHz mono float whisper at about -43 dBFS)"]
    fn real_detector_hears_quiet_whispers() {
        let mut session = load(Path::new(&std::env::var("HEX_VAD_MODEL").unwrap())).unwrap();
        assert!(detect(&mut session, &clip("HEX_VAD_WHISPER_WAV")).unwrap());
    }
}
