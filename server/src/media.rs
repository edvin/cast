use serde_json::Value;
use std::path::Path;
use std::process::Command;

/// Check if ffprobe is available in PATH.
pub fn is_ffprobe_available() -> bool {
    Command::new("ffprobe")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// Check if ffmpeg is available in PATH.
pub fn is_ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// Run ffprobe to get the video duration in seconds.
///
/// Returns `None` if ffprobe is not available, the command fails, or the
/// duration field cannot be parsed.
pub fn probe_duration(path: &Path) -> Option<f64> {
    let output = Command::new("ffprobe")
        .args(["-v", "quiet", "-print_format", "json", "-show_format"])
        .arg(path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let json: Value = serde_json::from_slice(&output.stdout).ok()?;
    json["format"]["duration"].as_str().and_then(|s| s.parse::<f64>().ok())
}

/// Extract a single frame from a video and save it as a JPEG thumbnail.
///
/// The `timestamp_secs` parameter controls which frame to extract. A good
/// default is 10% into the video or 30 seconds, whichever is less — the caller
/// can compute that via [`probe_duration`].
pub fn generate_thumbnail(video_path: &Path, output_path: &Path, timestamp_secs: f64) -> Result<(), std::io::Error> {
    let status = Command::new("ffmpeg")
        .args(["-ss", &format!("{timestamp_secs:.2}")])
        .arg("-i")
        .arg(video_path)
        .args(["-vframes", "1", "-q:v", "2", "-y"])
        .arg(output_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("ffmpeg exited with status {status}")))
    }
}
