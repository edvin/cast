/// Convert SRT subtitle content to WebVTT format.
///
/// - Adds `WEBVTT` header
/// - Replaces `,` with `.` in timestamp lines (`-->`)
/// - Preserves everything else (sequence numbers, text, blank lines)
pub fn srt_to_webvtt(srt: &str) -> String {
    let mut out = String::from("WEBVTT\n\n");

    for line in srt.lines() {
        if line.contains(" --> ") {
            out.push_str(&line.replace(',', "."));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_basic_srt_with_two_entries() {
        let srt = "\
1
00:00:01,000 --> 00:00:04,000
Hello, world!

2
00:01:15,500 --> 00:01:18,750
Second subtitle.
";

        let vtt = srt_to_webvtt(srt);
        assert!(vtt.starts_with("WEBVTT\n\n"));
        assert!(vtt.contains("00:00:01.000 --> 00:00:04.000"));
        assert!(vtt.contains("00:01:15.500 --> 00:01:18.750"));
        assert!(vtt.contains("Hello, world!"));
        assert!(vtt.contains("Second subtitle."));
    }

    #[test]
    fn handles_windows_line_endings() {
        let srt = "1\r\n00:00:01,000 --> 00:00:04,000\r\nHello\r\n";

        let vtt = srt_to_webvtt(srt);
        assert!(vtt.starts_with("WEBVTT\n\n"));
        assert!(vtt.contains("00:00:01.000 --> 00:00:04.000"));
        assert!(vtt.contains("Hello"));
    }

    #[test]
    fn empty_input_produces_header_only() {
        let vtt = srt_to_webvtt("");
        assert_eq!(vtt, "WEBVTT\n\n");
    }
}
