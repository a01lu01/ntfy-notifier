//! Bounded framing for Server-Sent Events.
//!
//! The decoder deliberately stops at the SSE framing layer. It joins `data`
//! fields and leaves JSON parsing and application-specific validation to the
//! caller.

/// Maximum wire size of one SSE event.
///
/// The limit counts every byte in every non-empty line, including comments,
/// unknown fields, and that line's LF, CR, or CRLF terminator. The terminating
/// empty line is framing and is not counted. This definition makes an event of
/// exactly 64 KiB valid while rejecting the next byte, regardless of whether
/// the useful `data` field itself is small.
pub const MAX_EVENT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DropReason {
    TooLarge,
    InvalidUtf8,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum DecodeOutcome {
    Data(Vec<u8>),
    Dropped(DropReason),
}

/// Incrementally decodes SSE framing without retaining caller-owned chunks.
///
/// Call [`Decoder::next_event`] repeatedly for a received chunk until it
/// returns `None`, then await the next chunk. A successful call returns at most
/// one event and leaves any following bytes in `input`, so callers never need
/// to collect all decoded events in memory.
pub(crate) struct Decoder {
    // Non-empty lines are stored with a normalized LF. `wire_bytes` retains the
    // original LF/CR/CRLF byte count used for enforcing MAX_EVENT_BYTES.
    event: Vec<u8>,
    wire_bytes: usize,
    line_nonempty: bool,
    pending_cr: bool,
    discarding: bool,
    at_stream_start: bool,
}

impl Default for Decoder {
    fn default() -> Self {
        Self {
            event: Vec::new(),
            wire_bytes: 0,
            line_nonempty: false,
            pending_cr: false,
            discarding: false,
            at_stream_start: true,
        }
    }
}

impl Decoder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Consumes bytes until one meaningful event is available or `input` is
    /// exhausted.
    ///
    /// Empty events and events containing only valid comments/unknown fields
    /// are consumed internally. An event with at least one `data` field is
    /// returned even when its joined data is empty.
    pub(crate) fn next_event(&mut self, input: &mut &[u8]) -> Option<DecodeOutcome> {
        loop {
            if self.pending_cr {
                let &next = input.first()?;
                self.pending_cr = false;

                if next == b'\n' {
                    *input = &input[1..];
                    if let Some(outcome) = self.end_line(2) {
                        return Some(outcome);
                    }
                } else if let Some(outcome) = self.end_line(1) {
                    // `next` belongs to the following line (and possibly the
                    // following event), so leave it for the next call.
                    return Some(outcome);
                }
                continue;
            }

            let (&byte, remaining) = input.split_first()?;
            *input = remaining;

            match byte {
                b'\r' => self.pending_cr = true,
                b'\n' => {
                    if let Some(outcome) = self.end_line(1) {
                        return Some(outcome);
                    }
                }
                byte => self.push_content(byte),
            }
        }
    }

    /// Resolves a final bare CR at end-of-stream and discards any event that
    /// was not terminated by an empty line.
    pub(crate) fn finish(&mut self) -> Option<DecodeOutcome> {
        let outcome = if self.pending_cr {
            self.pending_cr = false;
            self.end_line(1)
        } else {
            None
        };

        if outcome.is_none() {
            self.reset_event();
        }
        outcome
    }

    fn push_content(&mut self, byte: u8) {
        self.line_nonempty = true;
        if self.discarding {
            return;
        }

        if self.wire_bytes == MAX_EVENT_BYTES {
            self.start_discarding();
            return;
        }

        self.event.push(byte);
        self.wire_bytes += 1;
    }

    fn end_line(&mut self, terminator_bytes: usize) -> Option<DecodeOutcome> {
        if self.line_nonempty {
            self.line_nonempty = false;
            if self.discarding {
                return None;
            }

            if terminator_bytes > MAX_EVENT_BYTES - self.wire_bytes {
                self.start_discarding();
                return None;
            }

            self.wire_bytes += terminator_bytes;
            // Normalize all supported line endings for simple field parsing.
            // This cannot exceed the wire limit because one normalized LF is
            // never larger than the original terminator.
            self.event.push(b'\n');
            return None;
        }

        if self.discarding {
            self.at_stream_start = false;
            self.reset_event();
            return Some(DecodeOutcome::Dropped(DropReason::TooLarge));
        }

        self.finish_event()
    }

    fn finish_event(&mut self) -> Option<DecodeOutcome> {
        let event = std::mem::take(&mut self.event);
        self.wire_bytes = 0;
        let at_stream_start = std::mem::replace(&mut self.at_stream_start, false);

        let text = match std::str::from_utf8(&event) {
            Ok(text) => text,
            Err(_) => return Some(DecodeOutcome::Dropped(DropReason::InvalidUtf8)),
        };
        let text = if at_stream_start {
            text.strip_prefix('\u{feff}').unwrap_or(text)
        } else {
            text
        };

        let mut data = Vec::new();
        let mut saw_data = false;
        for line in text.split_terminator('\n') {
            if line.starts_with(':') {
                continue;
            }

            let (field, value) = match line.split_once(':') {
                Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
                None => (line, ""),
            };
            if field != "data" {
                continue;
            }

            if saw_data {
                data.push(b'\n');
            }
            saw_data = true;
            data.extend_from_slice(value.as_bytes());
        }

        saw_data.then_some(DecodeOutcome::Data(data))
    }

    fn start_discarding(&mut self) {
        self.event.clear();
        self.wire_bytes = 0;
        self.discarding = true;
    }

    fn reset_event(&mut self) {
        self.event.clear();
        self.wire_bytes = 0;
        self.line_nonempty = false;
        self.pending_cr = false;
        self.discarding = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn consume(decoder: &mut Decoder, chunk: &[u8]) -> Vec<DecodeOutcome> {
        let mut input = chunk;
        let mut outcomes = Vec::new();
        while !input.is_empty() {
            let before = input.len();
            if let Some(outcome) = decoder.next_event(&mut input) {
                outcomes.push(outcome);
            }
            if input.len() == before {
                // A bare CR can complete an event before the first byte of the
                // next event is consumed. The following iteration must consume
                // that byte, so two consecutive stalls would be a decoder bug.
                let stalled_at = input.len();
                if let Some(outcome) = decoder.next_event(&mut input) {
                    outcomes.push(outcome);
                }
                assert!(input.len() < stalled_at, "decoder made no progress");
            }
        }
        outcomes
    }

    fn consume_chunks(chunks: &[&[u8]]) -> Vec<DecodeOutcome> {
        let mut decoder = Decoder::new();
        let mut outcomes = Vec::new();
        for chunk in chunks {
            outcomes.extend(consume(&mut decoder, chunk));
        }
        outcomes
    }

    #[test]
    fn waits_for_the_blank_line_across_fragmented_chunks() {
        let mut decoder = Decoder::new();
        assert!(consume(&mut decoder, b"da").is_empty());
        assert!(consume(&mut decoder, b"ta: hel").is_empty());
        assert!(consume(&mut decoder, b"lo\n").is_empty());
        assert_eq!(
            consume(&mut decoder, b"\n"),
            vec![DecodeOutcome::Data(b"hello".to_vec())]
        );
    }

    #[test]
    fn decodes_the_same_event_at_every_two_chunk_split() {
        let wire = b": comment\r\nevent: message\rdata: first\ndata: second\r\nignored: value\n\n";
        for split in 0..=wire.len() {
            assert_eq!(
                consume_chunks(&[&wire[..split], &wire[split..]]),
                vec![DecodeOutcome::Data(b"first\nsecond".to_vec())],
                "failed at split {split}"
            );
        }

        let one_byte_chunks: Vec<&[u8]> = wire.chunks(1).collect();
        assert_eq!(
            consume_chunks(&one_byte_chunks),
            vec![DecodeOutcome::Data(b"first\nsecond".to_vec())]
        );
    }

    #[test]
    fn supports_lf_crlf_and_bare_cr_in_one_stream() {
        let outcomes = consume_chunks(&[
            b"data: lf\n\n",
            b"data: crlf\r",
            b"\n\r",
            b"\ndata: cr\r",
            b"\rdata: tail\n\n",
        ]);
        assert_eq!(
            outcomes,
            vec![
                DecodeOutcome::Data(b"lf".to_vec()),
                DecodeOutcome::Data(b"crlf".to_vec()),
                DecodeOutcome::Data(b"cr".to_vec()),
                DecodeOutcome::Data(b"tail".to_vec()),
            ]
        );
    }

    #[test]
    fn joins_data_fields_with_lf_and_keeps_an_explicit_empty_data_field() {
        assert_eq!(
            consume_chunks(&[b"data:first\ndata: second\ndata\n\ndata:\n\n"]),
            vec![
                DecodeOutcome::Data(b"first\nsecond\n".to_vec()),
                DecodeOutcome::Data(Vec::new()),
            ]
        );
    }

    #[test]
    fn comments_unknown_fields_and_empty_events_do_not_emit_data() {
        assert!(consume_chunks(&[b"\n: keepalive\nretry: 10\nunknown\n\n"]).is_empty());
    }

    #[test]
    fn accepts_exact_wire_limit_and_rejects_one_more_byte() {
        // `data:` + payload + LF is the counted event; the final LF is the
        // uncounted blank-line delimiter.
        let exact_payload = vec![b'x'; MAX_EVENT_BYTES - b"data:\n".len()];
        let mut exact = b"data:".to_vec();
        exact.extend_from_slice(&exact_payload);
        exact.extend_from_slice(b"\n\n");
        assert_eq!(
            consume_chunks(&[&exact]),
            vec![DecodeOutcome::Data(exact_payload)]
        );

        let oversized_payload = vec![b'x'; MAX_EVENT_BYTES - b"data:\n".len() + 1];
        let mut oversized = b"data:".to_vec();
        oversized.extend_from_slice(&oversized_payload);
        oversized.extend_from_slice(b"\n\n");
        assert_eq!(
            consume_chunks(&[&oversized]),
            vec![DecodeOutcome::Dropped(DropReason::TooLarge)]
        );
    }

    #[test]
    fn comments_and_unknown_fields_count_toward_the_wire_limit() {
        let mut wire = vec![b'x'; MAX_EVENT_BYTES - 1];
        wire.extend_from_slice(b"\n\n");
        assert!(consume_chunks(&[&wire]).is_empty());

        let mut oversized = vec![b'x'; MAX_EVENT_BYTES];
        oversized.extend_from_slice(b"\n\n");
        assert_eq!(
            consume_chunks(&[&oversized]),
            vec![DecodeOutcome::Dropped(DropReason::TooLarge)]
        );
    }

    #[test]
    fn continuous_line_storage_stays_bounded() {
        let mut decoder = Decoder::new();
        let chunk = vec![b'x'; MAX_EVENT_BYTES];
        for _ in 0..16 {
            assert!(consume(&mut decoder, &chunk).is_empty());
            assert!(decoder.event.len() <= MAX_EVENT_BYTES);
            assert!(decoder.event.capacity() <= MAX_EVENT_BYTES);
        }
        assert!(decoder.discarding);
        assert!(decoder.event.is_empty());
    }

    #[test]
    fn oversized_tail_cannot_masquerade_as_a_valid_event() {
        let mut wire = b"data:".to_vec();
        wire.extend(std::iter::repeat_n(b'x', MAX_EVENT_BYTES));
        wire.extend_from_slice(b"\ndata: attacker-controlled tail\n\ndata: safe\n\n");

        assert_eq!(
            consume_chunks(&[&wire]),
            vec![
                DecodeOutcome::Dropped(DropReason::TooLarge),
                DecodeOutcome::Data(b"safe".to_vec()),
            ]
        );
    }

    #[test]
    fn recovers_after_oversized_event_in_the_same_input() {
        let mut wire = vec![b'x'; MAX_EVENT_BYTES + 1];
        wire.extend_from_slice(b"\n\ndata: recovered\n\n");
        assert_eq!(
            consume_chunks(&[&wire]),
            vec![
                DecodeOutcome::Dropped(DropReason::TooLarge),
                DecodeOutcome::Data(b"recovered".to_vec()),
            ]
        );
    }

    #[test]
    fn drops_whole_invalid_utf8_event_and_recovers_without_raw_data() {
        let wire = b"data: valid prefix\n: invalid \xff comment\n\ndata: recovered\n\n";
        assert_eq!(
            consume_chunks(&[wire]),
            vec![
                DecodeOutcome::Dropped(DropReason::InvalidUtf8),
                DecodeOutcome::Data(b"recovered".to_vec()),
            ]
        );
        assert_eq!(format!("{:?}", DropReason::InvalidUtf8), "InvalidUtf8");
    }

    #[test]
    fn valid_utf8_split_inside_a_codepoint_is_preserved() {
        let wire = "data: 你好\n\n".as_bytes();
        let split = wire.iter().position(|byte| *byte >= 0x80).unwrap() + 1;
        assert_eq!(
            consume_chunks(&[&wire[..split], &wire[split..]]),
            vec![DecodeOutcome::Data("你好".as_bytes().to_vec())]
        );
    }

    #[test]
    fn ignores_one_utf8_bom_at_stream_start() {
        assert_eq!(
            consume_chunks(&[
                b"\xef",
                b"\xbb\xbfdata: first\n\n\xef\xbb\xbfdata: second\n\n"
            ]),
            vec![DecodeOutcome::Data(b"first".to_vec())]
        );
    }

    #[test]
    fn finish_resolves_final_bare_cr_but_not_an_unterminated_event() {
        let mut decoder = Decoder::new();
        assert!(consume(&mut decoder, b"data: complete\r\r").is_empty());
        assert_eq!(
            decoder.finish(),
            Some(DecodeOutcome::Data(b"complete".to_vec()))
        );

        assert!(consume(&mut decoder, b"data: incomplete\r").is_empty());
        assert_eq!(decoder.finish(), None);
        assert!(consume(&mut decoder, b"data: next\n\n")
            .contains(&DecodeOutcome::Data(b"next".to_vec())));
    }
}
