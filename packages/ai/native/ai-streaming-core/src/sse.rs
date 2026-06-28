//! Anthropic SSE framer: faithful port of anthropic-messages.ts L276-425
//! (ServerSentEvent, SseDecoderState, flushSseEvent, decodeSseLine, nextLineBreakIndex,
//! consumeLine, iterateSseMessages) over ordered byte chunks.
//!
//! JS accumulates a UTF-16 string buffer fed by `TextDecoder.decode(value, {stream:true})`,
//! which holds incomplete trailing multibyte bytes across reads. We mirror that with an
//! incremental UTF-8 decoder, then run the line scanner on the decoded string. Line breaks
//! (\r, \n) and field delimiters (:) are ASCII, so byte offsets coincide with char boundaries.

#[derive(Debug, Clone)]
pub struct ServerSentEvent {
	pub event: Option<String>,
	pub data: String,
	#[allow(dead_code)]
	pub raw: Vec<String>,
}

struct SseDecoderState {
	event: Option<String>,
	data: Vec<String>,
	raw: Vec<String>,
}

impl SseDecoderState {
	fn new() -> Self {
		SseDecoderState { event: None, data: Vec::new(), raw: Vec::new() }
	}
}

fn flush_sse_event(state: &mut SseDecoderState) -> Option<ServerSentEvent> {
	let event_falsy = match &state.event {
		None => true,
		Some(s) => s.is_empty(),
	};
	if event_falsy && state.data.is_empty() {
		return None;
	}
	let ev = ServerSentEvent {
		event: state.event.clone(),
		data: state.data.join("\n"),
		raw: state.raw.clone(),
	};
	state.event = None;
	state.data = Vec::new();
	state.raw = Vec::new();
	Some(ev)
}

fn decode_sse_line(line: &str, state: &mut SseDecoderState) -> Option<ServerSentEvent> {
	if line.is_empty() {
		return flush_sse_event(state);
	}
	state.raw.push(line.to_string());
	if line.starts_with(':') {
		return None;
	}
	let delimiter = line.find(':');
	let (field_name, value_raw) = match delimiter {
		None => (line, ""),
		Some(i) => (&line[..i], &line[i + 1..]),
	};
	let value = value_raw.strip_prefix(' ').unwrap_or(value_raw);
	if field_name == "event" {
		state.event = Some(value.to_string());
	} else if field_name == "data" {
		state.data.push(value.to_string());
	}
	None
}

fn next_line_break_index(text: &str) -> Option<usize> {
	let cr = text.find('\r');
	let nl = text.find('\n');
	match (cr, nl) {
		(None, None) => None,
		(None, Some(n)) => Some(n),
		(Some(c), None) => Some(c),
		(Some(c), Some(n)) => Some(c.min(n)),
	}
}

/// Returns (line, rest) or None if there is no complete line yet.
fn consume_line(text: &str) -> Option<(String, String)> {
	let lb = next_line_break_index(text)?;
	let bytes = text.as_bytes();
	let mut next = lb + 1;
	if bytes[lb] == b'\r' && bytes.get(next) == Some(&b'\n') {
		next += 1;
	}
	Some((text[..lb].to_string(), text[next..].to_string()))
}

/// Incremental UTF-8 decoder mirroring TextDecoder: holds incomplete trailing bytes across calls,
/// replaces genuinely invalid sequences with U+FFFD.
struct Utf8Stream {
	pending: Vec<u8>,
}

impl Utf8Stream {
	fn new() -> Self {
		Utf8Stream { pending: Vec::new() }
	}

	fn decode(&mut self, bytes: &[u8]) -> String {
		self.pending.extend_from_slice(bytes);
		let mut out = String::new();
		loop {
			match std::str::from_utf8(&self.pending) {
				Ok(s) => {
					out.push_str(s);
					self.pending.clear();
					break;
				}
				Err(e) => {
					let valid = e.valid_up_to();
					if valid > 0 {
						out.push_str(std::str::from_utf8(&self.pending[..valid]).unwrap());
					}
					match e.error_len() {
						None => {
							// Incomplete multibyte sequence at the end: keep it for the next chunk.
							self.pending.drain(..valid);
							break;
						}
						Some(bad) => {
							// Invalid sequence: emit a replacement char and skip it.
							out.push('\u{FFFD}');
							self.pending.drain(..valid + bad);
						}
					}
				}
			}
		}
		out
	}

	fn flush(&mut self) -> String {
		if self.pending.is_empty() {
			return String::new();
		}
		let out = String::from_utf8_lossy(&self.pending).into_owned();
		self.pending.clear();
		out
	}
}

/// Port of iterateSseMessages: ordered byte chunks -> ordered ServerSentEvents.
pub fn parse_sse(chunks: &[Vec<u8>]) -> Vec<ServerSentEvent> {
	let mut decoder = Utf8Stream::new();
	let mut state = SseDecoderState::new();
	let mut buffer = String::new();
	let mut out: Vec<ServerSentEvent> = Vec::new();

	for chunk in chunks {
		buffer.push_str(&decoder.decode(chunk));
		while let Some((line, rest)) = consume_line(&buffer) {
			buffer = rest;
			if let Some(ev) = decode_sse_line(&line, &mut state) {
				out.push(ev);
			}
		}
	}

	buffer.push_str(&decoder.flush());
	while let Some((line, rest)) = consume_line(&buffer) {
		buffer = rest;
		if let Some(ev) = decode_sse_line(&line, &mut state) {
			out.push(ev);
		}
	}

	if !buffer.is_empty() {
		if let Some(ev) = decode_sse_line(&buffer, &mut state) {
			out.push(ev);
		}
	}

	if let Some(ev) = flush_sse_event(&mut state) {
		out.push(ev);
	}

	out
}
