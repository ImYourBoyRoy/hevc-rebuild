//! Sampled picture-quality check (SSIM) used before an output replaces its source.
//!
//! The rebuild encodes with fixed settings and validates streams, metadata and
//! HDR, but until now nothing compared the *picture* with the original. This
//! module does that on a handful of short windows spread across the file, so the
//! cost stays small next to the encode itself.
//!
//! What the score can and cannot say. SSIM is a good detector of the failures
//! that matter in an unattended batch: a wrong filter, a broken decode, frames
//! out of sync, a catastrophically low bitrate. It is not proof that nothing is
//! visible: very grainy sources can score lower than they look, and banding in
//! dark gradients can score higher than it looks. So the default is a *floor*
//! that only a real regression falls under, plus the scores in every report.
//!
//! The comparison is fair by construction: both sides are converted to the same
//! 10-bit pixel format, and when the rebuild deinterlaced the picture the source
//! is deinterlaced with the same filter first, so interlacing is not scored as
//! damage.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

/// Why a comparison stopped without a score.
pub(crate) enum GateError {
    /// The user stopped the run while FFmpeg was measuring.
    Interrupted,
    /// FFmpeg could not produce a score (message for the log and report).
    Failed(String),
}

/// What a rebuild does with the score.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum GateMode {
    /// No measurement.
    Off,
    /// Measure and record the score; never block the install.
    Report,
    /// Measure and refuse to install an output that falls under the floor. The
    /// original stays untouched.
    #[default]
    Enforce,
}

impl GateMode {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        match text {
            "off" => Ok(Self::Off),
            "report" => Ok(Self::Report),
            "enforce" => Ok(Self::Enforce),
            other => Err(format!(
                "Quality gate must be off, report or enforce, not {other:?}."
            )),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Report => "report",
            Self::Enforce => "enforce",
        }
    }
}

/// Mean SSIM below this is a real regression for the app's default encodes.
pub(crate) const DEFAULT_MIN_SSIM: f64 = 0.95;
/// The worst single window may sit this far under the floor (a dark or noisy
/// scene scores lower without being damaged).
const WINDOW_TOLERANCE: f64 = 0.03;
const SAMPLE_SECONDS: f64 = 6.0;
const WHOLE_CLIP_SECONDS: f64 = 60.0;
const WINDOW_FRACTIONS: [f64; 6] = [0.08, 0.22, 0.36, 0.50, 0.64, 0.78];

/// Valid range for a user-supplied floor: SSIM is at most 1, and a floor under
/// 0.5 would accept almost anything.
pub(crate) fn validate_min_ssim(value: f64) -> Result<f64, String> {
    if value.is_finite() && (0.5..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err("Minimum SSIM must be a number between 0.5 and 1.0.".into())
    }
}

/// One measured window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WindowScore {
    pub(crate) start: f64,
    pub(crate) length: f64,
    /// SSIM over all planes, 0..=1.
    pub(crate) ssim: f64,
}

/// The decision on a set of window scores.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Verdict {
    pub(crate) passed: bool,
    pub(crate) mean: f64,
    pub(crate) lowest: f64,
    pub(crate) windows: usize,
    pub(crate) floor: f64,
}

impl Verdict {
    pub(crate) fn summary(&self) -> String {
        format!(
            "SSIM {:.4} average, {:.4} lowest across {} sample window{} (floor {:.2})",
            self.mean,
            self.lowest,
            self.windows,
            if self.windows == 1 { "" } else { "s" },
            self.floor
        )
    }
}

/// Passes when the average clears the floor and no single window falls more
/// than the tolerance below it. `None` when there is nothing to judge.
pub(crate) fn judge(scores: &[WindowScore], floor: f64) -> Option<Verdict> {
    if scores.is_empty() {
        return None;
    }
    let mean = scores.iter().map(|score| score.ssim).sum::<f64>() / scores.len() as f64;
    let lowest = scores
        .iter()
        .map(|score| score.ssim)
        .fold(f64::INFINITY, f64::min);
    Some(Verdict {
        passed: mean >= floor && lowest >= floor - WINDOW_TOLERANCE,
        mean,
        lowest,
        windows: scores.len(),
        floor,
    })
}

/// Windows to compare: the whole clip when short, otherwise six short windows
/// spread through the middle of the file (credits and logos at the ends say
/// little about the picture).
pub(crate) fn rebuild_windows(duration: f64) -> Vec<(f64, f64)> {
    if !duration.is_finite() || duration <= 0.0 {
        return vec![(0.0, SAMPLE_SECONDS)];
    }
    if duration <= WHOLE_CLIP_SECONDS {
        return vec![(0.0, duration)];
    }
    WINDOW_FRACTIONS
        .iter()
        .map(|fraction| {
            let start =
                (duration * fraction - SAMPLE_SECONDS / 2.0).clamp(0.0, duration - SAMPLE_SECONDS);
            (start, SAMPLE_SECONDS)
        })
        .collect()
}

/// Whether the rebuild deinterlaced this video track. Mirrors the encode plan's
/// rule so the comparison applies the same filter to the source.
pub(crate) fn rebuild_deinterlaced(
    track_interlaced: bool,
    force_deinterlace: bool,
    disable_deinterlace: bool,
) -> bool {
    force_deinterlace || (track_interlaced && !disable_deinterlace)
}

/// The two streams being compared.
pub(crate) struct Comparison<'a> {
    pub(crate) source: &'a Path,
    pub(crate) source_stream_index: i64,
    pub(crate) output: &'a Path,
    /// Position of the compared video among the output's video streams.
    pub(crate) output_video_ordinal: usize,
    /// The rebuild deinterlaced the picture, so the source is deinterlaced the
    /// same way before it is scored.
    pub(crate) deinterlace: bool,
}

pub(crate) fn comparison_filter(comparison: &Comparison<'_>) -> String {
    let prepare = if comparison.deinterlace {
        "bwdif=mode=send_frame:parity=auto:deint=interlaced,"
    } else {
        ""
    };
    format!(
        "[0:{}]{prepare}format=yuv420p10le,setpts=PTS-STARTPTS[a];\
         [1:v:{}]format=yuv420p10le,setpts=PTS-STARTPTS[b];[a][b]ssim",
        comparison.source_stream_index, comparison.output_video_ordinal
    )
}

/// Pull the `All:` value from an FFmpeg SSIM summary line.
pub(crate) fn parse_ssim_all(line: &str) -> Option<f64> {
    let rest = line.split("SSIM ").nth(1)?;
    let value = rest.split("All:").nth(1)?;
    value
        .split(|character: char| character.is_whitespace() || character == '(')
        .next()?
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

/// Measures every window. Both inputs are cut with the same `-ss`/`-t`, so the
/// frames line up one to one.
pub(crate) fn measure_windows(
    ffmpeg: &str,
    comparison: &Comparison<'_>,
    windows: &[(f64, f64)],
    mut on_child: impl FnMut(Option<u32>),
) -> Result<Vec<WindowScore>, GateError> {
    let filter = comparison_filter(comparison);
    let mut scores = Vec::with_capacity(windows.len());
    for &(start, length) in windows {
        let mut command = Command::new(ffmpeg);
        command
            .args(["-hide_banner", "-nostdin", "-nostats", "-loglevel", "info"])
            .args([
                "-ss",
                &format!("{start:.3}"),
                "-t",
                &format!("{length:.3}"),
                "-i",
            ])
            .arg(comparison.source)
            .args([
                "-ss",
                &format!("{start:.3}"),
                "-t",
                &format!("{length:.3}"),
                "-i",
            ])
            .arg(comparison.output)
            .args(["-filter_complex", &filter])
            .args(["-an", "-sn", "-dn", "-f", "null", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = crate::process_output::spawn_owned(&mut command).map_err(|error| {
            GateError::Failed(format!("Could not start the SSIM check: {error}"))
        })?;
        on_child(Some(child.id()));
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| GateError::Failed("Could not read the SSIM check output.".into()))?;
        let reader = std::thread::spawn(move || {
            let mut ssim = None;
            let mut tail: Vec<String> = Vec::new();
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(value) = parse_ssim_all(&line) {
                    ssim = Some(value);
                }
                tail.push(line);
                if tail.len() > 12 {
                    tail.remove(0);
                }
            }
            (ssim, tail)
        });
        let status = child.wait();
        on_child(None);
        let (ssim, tail) = reader.join().map_err(|_| {
            GateError::Failed("The SSIM output reader stopped unexpectedly.".into())
        })?;
        let status = status.map_err(|error| {
            GateError::Failed(format!("The SSIM check did not finish: {error}"))
        })?;
        if status.code().is_none() {
            return Err(GateError::Interrupted);
        }
        if !status.success() {
            return Err(GateError::Failed(format!(
                "The SSIM check failed: {}",
                tail.join(" | ")
            )));
        }
        let ssim = ssim.ok_or_else(|| {
            GateError::Failed("The SSIM check produced no similarity score.".into())
        })?;
        scores.push(WindowScore {
            start,
            length,
            ssim,
        });
    }
    if scores.is_empty() {
        return Err(GateError::Failed(
            "The SSIM check had nothing to compare.".into(),
        ));
    }
    Ok(scores)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(ssim: f64) -> WindowScore {
        WindowScore {
            start: 0.0,
            length: 6.0,
            ssim,
        }
    }

    #[test]
    fn modes_parse_and_round_trip() {
        for mode in [GateMode::Off, GateMode::Report, GateMode::Enforce] {
            assert_eq!(GateMode::parse(mode.as_str()), Ok(mode));
        }
        assert!(GateMode::parse("strict").is_err());
    }

    #[test]
    fn the_floor_must_be_a_sane_ssim() {
        assert_eq!(validate_min_ssim(0.95), Ok(0.95));
        assert_eq!(validate_min_ssim(1.0), Ok(1.0));
        for bad in [0.0, 0.49, 1.01, f64::NAN, f64::INFINITY] {
            assert!(validate_min_ssim(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_good_encode_passes_and_a_broken_one_fails() {
        let good = judge(&[score(0.99), score(0.985), score(0.992)], 0.95).unwrap();
        assert!(good.passed);
        assert!((good.mean - 0.989_000).abs() < 1e-3);
        assert_eq!(good.lowest, 0.985);

        let broken = judge(&[score(0.99), score(0.62), score(0.99)], 0.95).unwrap();
        assert!(
            !broken.passed,
            "one wrecked window fails even with a fine mean"
        );

        let soft = judge(&[score(0.93), score(0.93), score(0.93)], 0.95).unwrap();
        assert!(!soft.passed, "a uniformly low average fails");
    }

    #[test]
    fn one_slightly_low_window_is_tolerated_when_the_average_is_fine() {
        // A dark, grainy scene scores lower without being damaged.
        let verdict = judge(&[score(0.99), score(0.99), score(0.93)], 0.95).unwrap();
        assert!(verdict.passed, "{verdict:?}");
        let verdict = judge(&[score(0.99), score(0.99), score(0.91)], 0.95).unwrap();
        assert!(!verdict.passed, "more than the tolerance under the floor");
    }

    #[test]
    fn nothing_measured_means_no_verdict() {
        assert_eq!(judge(&[], 0.95), None);
    }

    #[test]
    fn the_summary_reads_plainly() {
        let verdict = judge(&[score(0.99)], 0.95).unwrap();
        assert_eq!(
            verdict.summary(),
            "SSIM 0.9900 average, 0.9900 lowest across 1 sample window (floor 0.95)"
        );
        let verdict = judge(&[score(0.99), score(0.98)], 0.95).unwrap();
        assert!(verdict.summary().contains("2 sample windows"));
    }

    #[test]
    fn short_clips_are_compared_whole_and_long_ones_in_six_inner_windows() {
        assert_eq!(rebuild_windows(45.0), vec![(0.0, 45.0)]);
        assert_eq!(rebuild_windows(f64::NAN), vec![(0.0, 6.0)]);
        let windows = rebuild_windows(7200.0);
        assert_eq!(windows.len(), 6);
        assert!(windows[0].0 > 300.0, "skips the opening logos");
        assert!(
            windows[5].0 + windows[5].1 < 6000.0,
            "skips the end credits"
        );
        assert!(windows.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert!(rebuild_windows(61.0)
            .iter()
            .all(|(start, length)| *start >= 0.0 && start + length <= 61.0));
    }

    #[test]
    fn deinterlacing_follows_the_encode_rule() {
        assert!(rebuild_deinterlaced(true, false, false));
        assert!(!rebuild_deinterlaced(false, false, false));
        assert!(
            !rebuild_deinterlaced(true, false, true),
            "disabled by the user"
        );
        assert!(rebuild_deinterlaced(false, true, false), "forced");
        assert!(rebuild_deinterlaced(true, true, true), "force wins");
    }

    #[test]
    fn the_filter_matches_pixel_format_and_adds_the_same_deinterlace() {
        let source = Path::new("/m/a.mkv");
        let output = Path::new("/m/.work/out.mkv");
        let plain = comparison_filter(&Comparison {
            source,
            source_stream_index: 2,
            output,
            output_video_ordinal: 1,
            deinterlace: false,
        });
        assert_eq!(
            plain,
            "[0:2]format=yuv420p10le,setpts=PTS-STARTPTS[a];\
             [1:v:1]format=yuv420p10le,setpts=PTS-STARTPTS[b];[a][b]ssim"
        );
        let deinterlaced = comparison_filter(&Comparison {
            source,
            source_stream_index: 0,
            output,
            output_video_ordinal: 0,
            deinterlace: true,
        });
        assert!(deinterlaced
            .starts_with("[0:0]bwdif=mode=send_frame:parity=auto:deint=interlaced,format="));
    }

    #[test]
    fn the_ssim_summary_line_is_parsed() {
        assert_eq!(
            parse_ssim_all(
                "[Parsed_ssim_5 @ 0x1] SSIM Y:0.99 (20.0) U:0.98 (17.0) V:0.98 (17.0) All:0.9876 (19.1)"
            ),
            Some(0.9876)
        );
        assert_eq!(parse_ssim_all("frame=1 fps=0"), None);
    }
}
