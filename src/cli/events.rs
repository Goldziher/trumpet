//! `trumpet events tail` subcommand — stream SSE events from the daemon.

use crate::error::Result;

use super::client::unix_get_streaming;

/// Arguments for `trumpet events tail`.
#[derive(clap::Args, Debug)]
pub struct EventsTailArgs {
    /// Comma-separated event-type filter (e.g. `agent_registered,task_created`).
    /// When omitted, all events are printed.
    #[arg(long)]
    pub filter: Option<String>,
}

/// Connect to the daemon SSE stream and print events until the connection
/// closes or the process is interrupted.
///
/// # Errors
///
/// Returns [`crate::error::Error::ConnectionRefused`] when the daemon is
/// unreachable.
pub async fn run_tail(args: EventsTailArgs) -> Result<()> {
    let path = "/events";

    let filter: Option<Vec<String>> = args
        .filter
        .as_deref()
        .map(|f| f.split(',').map(str::trim).map(String::from).collect());

    let mut parser = SseLineParser::default();

    unix_get_streaming(path, move |chunk| {
        for frame in parser.feed(chunk) {
            let passes = filter.as_ref().is_none_or(|types| {
                frame
                    .event_type
                    .as_deref()
                    .map(|t| types.iter().any(|f| f == t))
                    .unwrap_or(false)
            });
            if !passes {
                continue;
            }
            if let Some(d) = frame.data {
                let label = frame.event_type.as_deref().unwrap_or("event");
                println!("[{label}] {d}");
            }
        }
    })
    .await
}

/// One assembled SSE event.
#[derive(Debug, Default)]
pub(crate) struct SseFrame {
    pub event_type: Option<String>,
    pub data: Option<String>,
}

/// Line-oriented SSE parser. Tolerates HTTP/1.1 chunked transfer encoding by
/// skipping any line it can't classify as `event:` or `data:` (chunk-size
/// lines look like `1a` and never match either prefix).
#[derive(Default)]
pub(crate) struct SseLineParser {
    line_buf: String,
    current: SseFrame,
}

impl SseLineParser {
    /// Feed a byte slice, return any complete frames the bytes terminated.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<SseFrame> {
        let chunk = String::from_utf8_lossy(bytes);
        let mut out = Vec::new();
        for ch in chunk.chars() {
            if ch == '\n' {
                let line = std::mem::take(&mut self.line_buf);
                let trimmed = line.trim_end_matches('\r');
                if trimmed.is_empty() {
                    if self.current.data.is_some() || self.current.event_type.is_some() {
                        out.push(std::mem::take(&mut self.current));
                    }
                } else if let Some(rest) = trimmed.strip_prefix("event:") {
                    self.current.event_type = Some(rest.trim().to_owned());
                } else if let Some(rest) = trimmed.strip_prefix("data:") {
                    self.current.data = Some(rest.trim().to_owned());
                }
            } else {
                self.line_buf.push(ch);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_emits_frame_on_blank_line() {
        let mut p = SseLineParser::default();
        let frames = p.feed(b"event: agent_registered\ndata: {\"id\":\"x\"}\n\n");
        assert_eq!(frames.len(), 1, "must emit exactly one frame");
        assert_eq!(frames[0].event_type.as_deref(), Some("agent_registered"));
        assert_eq!(frames[0].data.as_deref(), Some("{\"id\":\"x\"}"));
    }

    #[test]
    fn parser_tolerates_chunked_size_lines() {
        let mut p = SseLineParser::default();
        // Simulate HTTP/1.1 chunked transfer: "1a\r\nevent: x\ndata: y\n\n\r\n"
        let bytes = b"1a\r\nevent: x\ndata: y\n\n\r\n";
        let frames = p.feed(bytes);
        assert_eq!(frames.len(), 1, "chunk-size line must be skipped");
        assert_eq!(frames[0].event_type.as_deref(), Some("x"));
        assert_eq!(frames[0].data.as_deref(), Some("y"));
    }

    #[test]
    fn parser_handles_split_chunks() {
        let mut p = SseLineParser::default();
        let frames1 = p.feed(b"event: x\nda");
        assert!(frames1.is_empty(), "frame not yet complete");
        let frames2 = p.feed(b"ta: y\n\n");
        assert_eq!(
            frames2.len(),
            1,
            "frame completes once second chunk arrives"
        );
        assert_eq!(frames2[0].event_type.as_deref(), Some("x"));
        assert_eq!(frames2[0].data.as_deref(), Some("y"));
    }
}
