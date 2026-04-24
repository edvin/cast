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
///
/// Runs a short `cropdetect` pass over the same window first so baked-in
/// cinematic letterbox bars (common on 2.35:1 / 2.39:1 content delivered in a
/// 16:9 container) don't survive into the thumbnail.
///
/// On failure, the returned error carries the tail of ffmpeg's stderr so the
/// caller can log *why* it failed — useful for memoizing bad files.
pub fn generate_thumbnail(video_path: &Path, output_path: &Path, timestamp_secs: f64) -> Result<(), std::io::Error> {
    let crop = detect_crop(video_path, timestamp_secs);

    let mut cmd = ffmpeg_command();
    cmd.args(["-hide_banner", "-loglevel", "error"])
        .args(["-ss", &format!("{timestamp_secs:.2}")])
        .arg("-i")
        .arg(video_path);
    if let Some(ref filter) = crop {
        cmd.args(["-vf", filter]);
    }
    let output = cmd
        .args(["-vframes", "1", "-q:v", "2", "-y"])
        .arg(output_path)
        .stdout(std::process::Stdio::null())
        .output()?;

    if output.status.success() && output_path.exists() {
        return Ok(());
    }

    // Pull the last non-empty line of stderr as the reason — ffmpeg is chatty,
    // but the actionable bit is usually right at the end.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reason = stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("ffmpeg failed");
    Err(std::io::Error::other(format!(
        "ffmpeg exited with {}: {reason}",
        output.status
    )))
}

/// Analyse ~2 seconds of video starting at `timestamp_secs` to detect any
/// baked-in letterbox bars. Returns a `crop=W:H:X:Y` filter string when
/// cropdetect reports a crop that's meaningfully tighter than the source
/// frame, otherwise `None`.
///
/// The pass is cheap — 2 seconds of decode with no encode, `-loglevel info`
/// just to keep `[Parsed_cropdetect_*]` lines on stderr. We deliberately
/// don't fail the whole thumbnail if this probe returns garbage: a dark
/// scene, a solid-color intro, or ffmpeg flakiness all silently skip the
/// crop and we fall back to extracting the native frame.
fn detect_crop(video_path: &Path, timestamp_secs: f64) -> Option<String> {
    let output = ffmpeg_command()
        .args(["-hide_banner", "-loglevel", "info"])
        .args(["-ss", &format!("{timestamp_secs:.2}")])
        .args(["-t", "2"])
        .arg("-i")
        .arg(video_path)
        .args(["-vf", "cropdetect=limit=24:round=2:reset_count=0"])
        .args(["-an", "-sn", "-f", "null", "-"])
        .stdout(std::process::Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    // cropdetect emits lines like:
    //   [Parsed_cropdetect_0 @ 0x...] x1:0 x2:1919 y1:138 y2:941 w:1920 h:804 x:0 y:138 ... crop=1920:804:0:138
    // We want the *last* such crop value — cropdetect converges as it sees
    // more frames, and bright mid-scene frames give a tighter answer than
    // the dark lead-ins the probe may start on.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let crop_expr = stderr
        .lines()
        .rev()
        .find_map(|line| {
            line.split("crop=")
                .nth(1)
                .map(|s| s.split_whitespace().next().unwrap_or(""))
        })
        .filter(|s| !s.is_empty())?;

    // Parse `W:H:X:Y`. Only apply the crop if it actually trims something —
    // otherwise we'd just slow thumbnail gen down without changing the output.
    let parts: Vec<&str> = crop_expr.split(':').collect();
    if parts.len() != 4 {
        return None;
    }
    let w: i32 = parts[0].parse().ok()?;
    let h: i32 = parts[1].parse().ok()?;
    let x: i32 = parts[2].parse().ok()?;
    let y: i32 = parts[3].parse().ok()?;
    if w <= 0 || h <= 0 {
        return None;
    }
    // Guard against degenerate detections on solid-black scenes — if cropdetect
    // thinks the content is a tiny sliver, fall back to the full frame.
    if h < 100 || w < 100 {
        return None;
    }
    if x == 0 && y == 0 {
        // Full frame — no bars to trim.
        return None;
    }

    Some(format!("crop={w}:{h}:{x}:{y}"))
}
