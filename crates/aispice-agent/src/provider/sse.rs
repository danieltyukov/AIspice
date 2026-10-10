//! A small server-sent events parser.
//!
//! Written by hand rather than pulled in because the providers need only the
//! `event` and `data` fields, and because it has to be lenient: some
//! OpenAI-compatible servers end lines with CRLF, omit the blank line after
//! the last event, or split a UTF-8 character across network chunks.
//!
//! It also has to be safe against a hostile stream: a line or an event
//! longer than `max_line` is an error rather than unbounded buffering, and
//! each byte is scanned for a newline once, so a long line arriving a byte at
//! a time costs linear time, not quadratic.

use super::ProviderError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug)]
pub(crate) struct SseParser {
    buf: Vec<u8>,
    /// Bytes at the start of `buf` already known to hold no newline.
    scanned: usize,
    event: Option<String>,
    data: Vec<String>,
    data_len: usize,
    max_line: usize,
}

impl SseParser {
    pub(crate) fn new(max_line: usize) -> Self {
        Self {
            buf: Vec::new(),
            scanned: 0,
            event: None,
            data: Vec::new(),
            data_len: 0,
            max_line,
        }
    }

    /// Feed bytes; returns every event completed by them.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, ProviderError> {
        let mut buf = std::mem::take(&mut self.buf);
        buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        let mut start = 0;
        let mut search = self.scanned;
        while let Some(offset) = buf[search..].iter().position(|&b| b == b'\n') {
            let end = search + offset;
            self.line(&buf[start..end], &mut out)?;
            start = end + 1;
            search = start;
        }
        buf.drain(..start);
        if buf.len() > self.max_line {
            return Err(self.too_long());
        }
        self.scanned = buf.len();
        self.buf = buf;
        Ok(out)
    }

    /// The body ended: flush a trailing line and any event left open.
    pub(crate) fn finish(&mut self) -> Result<Vec<SseEvent>, ProviderError> {
        let mut out = Vec::new();
        if !self.buf.is_empty() {
            let rest = std::mem::take(&mut self.buf);
            self.line(&rest, &mut out)?;
        }
        self.dispatch(&mut out);
        Ok(out)
    }

    fn too_long(&self) -> ProviderError {
        ProviderError::too_large("a line of the event stream", self.max_line)
    }

    fn line(&mut self, raw: &[u8], out: &mut Vec<SseEvent>) -> Result<(), ProviderError> {
        if raw.len() > self.max_line {
            return Err(self.too_long());
        }
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            self.dispatch(out);
            return Ok(());
        }
        if raw.starts_with(b":") {
            return Ok(());
        }
        let line = String::from_utf8_lossy(raw);
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line.as_ref(), ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => {
                self.data_len += value.len() + 1;
                if self.data_len > self.max_line {
                    return Err(ProviderError::too_large(
                        "an event of the event stream",
                        self.max_line,
                    ));
                }
                self.data.push(value.to_string());
            }
            _ => {}
        }
        Ok(())
    }

    fn dispatch(&mut self, out: &mut Vec<SseEvent>) {
        self.data_len = 0;
        if self.data.is_empty() {
            self.event = None;
            return;
        }
        out.push(SseEvent {
            event: self.event.take(),
            data: std::mem::take(&mut self.data).join("\n"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: usize = 1024 * 1024;

    fn ev(event: Option<&str>, data: &str) -> SseEvent {
        SseEvent {
            event: event.map(str::to_string),
            data: data.to_string(),
        }
    }

    #[test]
    fn parses_named_events_across_chunks() {
        let mut p = SseParser::new(MAX);
        let mut out = p.push(b"event: ping\ndata: {\"type\"").unwrap();
        assert!(out.is_empty());
        out.extend(p.push(b":\"ping\"}\n\n: comment\n\ndata: x\n\n").unwrap());
        assert_eq!(
            out,
            vec![ev(Some("ping"), "{\"type\":\"ping\"}"), ev(None, "x")]
        );
    }

    #[test]
    fn handles_crlf_multiline_and_missing_final_blank_line() {
        let mut p = SseParser::new(MAX);
        let mut out = p.push(b"data: a\r\ndata: b\r\n\r\ndata: [DONE]").unwrap();
        out.extend(p.finish().unwrap());
        assert_eq!(out, vec![ev(None, "a\nb"), ev(None, "[DONE]")]);
    }

    #[test]
    fn keeps_utf8_split_across_chunks() {
        let text = "data: \u{b5}F\n\n".as_bytes();
        let mut p = SseParser::new(MAX);
        let mut out = p.push(&text[..7]).unwrap();
        out.extend(p.push(&text[7..]).unwrap());
        assert_eq!(out, vec![ev(None, "\u{b5}F")]);
    }

    #[test]
    fn data_without_space_and_empty_events() {
        let mut p = SseParser::new(MAX);
        let out = p.push(b"event: x\n\ndata:{}\n\n").unwrap();
        assert_eq!(out, vec![ev(None, "{}")]);
    }

    #[test]
    fn rejects_an_oversized_line_whether_buffered_or_complete() {
        let mut p = SseParser::new(64);
        let err = p.push(&[b'a'; 100]).unwrap_err();
        assert!(matches!(err, ProviderError::TooLarge(_)), "{err}");

        let mut p = SseParser::new(64);
        let mut line = b"data: ".to_vec();
        line.extend_from_slice(&[b'x'; 100]);
        line.push(b'\n');
        assert!(matches!(p.push(&line), Err(ProviderError::TooLarge(_))));
    }

    #[test]
    fn rejects_an_oversized_event_made_of_short_lines() {
        let mut p = SseParser::new(64);
        let result = (0..20).try_for_each(|_| p.push(b"data: 0123456789\n").map(|_| ()));
        assert!(matches!(result, Err(ProviderError::TooLarge(_))));
    }

    #[test]
    fn a_long_line_in_tiny_chunks_is_linear() {
        let mut p = SseParser::new(MAX);
        let started = std::time::Instant::now();
        p.push(b"data: ").unwrap();
        for _ in 0..500_000 {
            p.push(b"x").unwrap();
        }
        let out = p.push(b"\n\n").unwrap();
        assert_eq!(out[0].data.len(), 500_000);
        // Quadratic scanning would take minutes here.
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
    }
}
