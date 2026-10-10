//! A small server-sent events parser.
//!
//! Written by hand rather than pulled in because the providers need only the
//! `event` and `data` fields, and because it has to be lenient: some
//! OpenAI-compatible servers end lines with CRLF, omit the blank line after
//! the last event, or split a UTF-8 character across network chunks.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Default)]
pub(crate) struct SseParser {
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    /// Feed bytes; returns every event completed by them.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line[..line.len() - 1]);
            self.line(line.strip_suffix('\r').unwrap_or(&line), &mut out);
        }
        out
    }

    /// The body ended: flush a trailing line and any event left open.
    pub(crate) fn finish(&mut self) -> Vec<SseEvent> {
        let mut out = Vec::new();
        if !self.buf.is_empty() {
            let rest = std::mem::take(&mut self.buf);
            let line = String::from_utf8_lossy(&rest);
            self.line(line.strip_suffix('\r').unwrap_or(&line), &mut out);
        }
        self.dispatch(&mut out);
        out
    }

    fn line(&mut self, line: &str, out: &mut Vec<SseEvent>) {
        if line.is_empty() {
            self.dispatch(out);
            return;
        }
        if line.starts_with(':') {
            return;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => self.data.push(value.to_string()),
            _ => {}
        }
    }

    fn dispatch(&mut self, out: &mut Vec<SseEvent>) {
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

    fn ev(event: Option<&str>, data: &str) -> SseEvent {
        SseEvent {
            event: event.map(str::to_string),
            data: data.to_string(),
        }
    }

    #[test]
    fn parses_named_events_across_chunks() {
        let mut p = SseParser::default();
        let mut out = p.push(b"event: ping\ndata: {\"type\"");
        assert!(out.is_empty());
        out.extend(p.push(b":\"ping\"}\n\n: comment\n\ndata: x\n\n"));
        assert_eq!(
            out,
            vec![ev(Some("ping"), "{\"type\":\"ping\"}"), ev(None, "x")]
        );
    }

    #[test]
    fn handles_crlf_multiline_and_missing_final_blank_line() {
        let mut p = SseParser::default();
        let mut out = p.push(b"data: a\r\ndata: b\r\n\r\ndata: [DONE]");
        out.extend(p.finish());
        assert_eq!(out, vec![ev(None, "a\nb"), ev(None, "[DONE]")]);
    }

    #[test]
    fn keeps_utf8_split_across_chunks() {
        let text = "data: \u{b5}F\n\n".as_bytes();
        let mut p = SseParser::default();
        let mut out = p.push(&text[..7]);
        out.extend(p.push(&text[7..]));
        assert_eq!(out, vec![ev(None, "\u{b5}F")]);
    }

    #[test]
    fn data_without_space_and_empty_events() {
        let mut p = SseParser::default();
        let out = p.push(b"event: x\n\ndata:{}\n\n");
        assert_eq!(out, vec![ev(None, "{}")]);
    }
}
