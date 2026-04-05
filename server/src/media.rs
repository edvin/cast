use serde_json::Value;
use std::path::Path;
use std::process::Command;

/// Create a Command that won't open a visible console window on Windows.
#[cfg(target_os = "windows")]
fn silent_command(program: &str) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut cmd = Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(not(target_os = "windows"))]
fn silent_command(program: &str) -> Command {
    Command::new(program)
}

/// Create a pre-configured Command for ffmpeg (hidden console on Windows).
pub fn ffmpeg_command() -> Command {
    silent_command(&ffmpeg_cmd())
}

/// Create a pre-configured Command for ffprobe (hidden console on Windows).
pub fn ffprobe_command() -> Command {
    silent_command(&ffprobe_cmd())
}

/// Find the full path to an ffmpeg/ffprobe binary.
/// Checks PATH first, then common install locations on Windows.
pub fn find_tool(name: &str) -> Option<String> {
    // Try PATH first
    if silent_command(name)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
    {
        return Some(name.to_string());
    }

    // Check common locations on Windows
    #[cfg(target_os = "windows")]
    {
        let candidates = [
            format!("C:\\ffmpeg\\bin\\{name}.exe"),
            format!("C:\\Program Files\\ffmpeg\\bin\\{name}.exe"),
            format!("C:\\Program Files (x86)\\ffmpeg\\bin\\{name}.exe"),
        ];
        // Also check next to the executable
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let local = dir.join(format!("{name}.exe"));
                if local.exists() {
                    return Some(local.to_string_lossy().to_string());
                }
            }
        }
        for path in &candidates {
            if std::path::Path::new(path).exists() {
                return Some(path.clone());
            }
        }
    }

    None
}

/// Check if ffprobe is available.
pub fn is_ffprobe_available() -> bool {
    find_tool("ffprobe").is_some()
}

/// Check if ffmpeg is available.
pub fn is_ffmpeg_available() -> bool {
    find_tool("ffmpeg").is_some()
}

/// Get the ffmpeg command name (might be a full path on Windows)
pub fn ffmpeg_cmd() -> String {
    find_tool("ffmpeg").unwrap_or_else(|| "ffmpeg".to_string())
}

/// Get the ffprobe command name
pub fn ffprobe_cmd() -> String {
    find_tool("ffprobe").unwrap_or_else(|| "ffprobe".to_string())
}

/// Run ffprobe to get the video duration in seconds.
///
/// Returns `None` if ffprobe is not available, the command fails, or the
/// duration field cannot be parsed.
pub fn probe_duration(path: &Path) -> Option<f64> {
    let output = ffprobe_command()
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
    let status = ffmpeg_command()
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
