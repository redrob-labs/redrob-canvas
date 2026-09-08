// SPDX-License-Identifier: GPL-3.0-or-later

use crate::{Error, Result};

const DEFAULT_MAX_EVENT_BYTES: usize = 1024 * 1024;

/// Incremental SSE `data:` field parser. It operates on bytes so UTF-8 and line
/// boundaries may be split across arbitrary transport chunks.
pub(crate) struct SseParser {
    line_buffer: Vec<u8>,
    data_lines: Vec<Vec<u8>>,
    event_bytes: usize,
    max_event_bytes: usize,
}

impl Default for SseParser {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_EVENT_BYTES)
    }
}

impl SseParser {
    pub(crate) fn new(max_event_bytes: usize) -> Self {
        Self {
            line_buffer: Vec::new(),
            data_lines: Vec::new(),
            event_bytes: 0,
            max_event_bytes,
        }
    }

    pub(crate) fn feed(&mut self, chunk: &[u8]) -> Result<Vec<String>> {
        let mut events = Vec::new();
        for &byte in chunk {
            if byte == b'\n' {
                let mut line = std::mem::take(&mut self.line_buffer);
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                self.process_line(line, &mut events)?;
            } else {
                self.line_buffer.push(byte);
                self.ensure_size(self.line_buffer.len())?;
            }
        }
        Ok(events)
    }

    pub(crate) fn finish(&mut self) -> Result<Vec<String>> {
        let mut events = Vec::new();
        if !self.line_buffer.is_empty() {
            let mut line = std::mem::take(&mut self.line_buffer);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.process_line(line, &mut events)?;
        }
        self.dispatch(&mut events)?;
        Ok(events)
    }

    fn process_line(&mut self, line: Vec<u8>, events: &mut Vec<String>) -> Result<()> {
        if line.is_empty() {
            return self.dispatch(events);
        }
        if line[0] == b':' {
            return Ok(());
        }

        let (field, mut value) = match line.iter().position(|byte| *byte == b':') {
            Some(index) => (&line[..index], &line[index + 1..]),
            None => (&line[..], &[][..]),
        };
        if value.first() == Some(&b' ') {
            value = &value[1..];
        }
        if field == b"data" {
            self.event_bytes = self
                .event_bytes
                .checked_add(value.len() + 1)
                .ok_or_else(|| Error::Sse("event size overflow".into()))?;
            self.ensure_size(self.event_bytes)?;
            self.data_lines.push(value.to_vec());
        }
        Ok(())
    }

    fn dispatch(&mut self, events: &mut Vec<String>) -> Result<()> {
        if self.data_lines.is_empty() {
            self.event_bytes = 0;
            return Ok(());
        }
        let data_lines = std::mem::take(&mut self.data_lines);
        let data = data_lines.join(&b'\n');
        self.event_bytes = 0;
        let data = String::from_utf8(data)
            .map_err(|_| Error::Sse("event data is not valid UTF-8".into()))?;
        events.push(data);
        Ok(())
    }

    fn ensure_size(&self, size: usize) -> Result<()> {
        if size > self.max_event_bytes {
            Err(Error::Sse(format!(
                "event exceeded {} byte limit",
                self.max_event_bytes
            )))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SseParser;

    #[test]
    fn parses_fragmented_crlf_multiline_and_comments() {
        let input = b": keepalive\r\ndata: {\"a\":\r\ndata: 1}\r\n\r\ndata: [DONE]\n\n";
        for split in 0..=input.len() {
            let mut parser = SseParser::default();
            let mut events = parser.feed(&input[..split]).unwrap();
            events.extend(parser.feed(&input[split..]).unwrap());
            events.extend(parser.finish().unwrap());
            assert_eq!(events, ["{\"a\":\n1}", "[DONE]"]);
        }
    }

    #[test]
    fn preserves_fragmented_utf8() {
        let input = "data: café\n\n".as_bytes();
        for chunk in input.chunks(1) {
            // Deliberately feed one byte at a time.
            let _ = chunk;
        }
        let mut parser = SseParser::default();
        let mut events = Vec::new();
        for chunk in input.chunks(1) {
            events.extend(parser.feed(chunk).unwrap());
        }
        assert_eq!(events, ["café"]);
    }
}
