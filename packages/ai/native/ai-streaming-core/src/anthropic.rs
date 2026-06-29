//! Faithful port of anthropic-messages.ts: iterateAnthropicEvents (L427-466), the stream()
//! assembly loop (L546-715), and mapStopReason (L1213-1239). Produces a normalized transcript
//! (the ordered AssistantMessageEvent sequence with per-event message snapshots, plus the final
//! message) as a JsVal, ready for canonical serialization against the TS oracle.
//!
//! Normalization is baked into the snapshot builder: timestamp and cost (non-deterministic /
//! model-rate-table-derived) and the streaming scratch fields (index, partialJson) are omitted;
//! everything else — event order, contentIndex, text/thinking content, and per-delta parsed
//! tool arguments — is preserved exactly.

use serde_json::Value;

use crate::partial_json::{
	jint, js_num, js_obj, js_str, parse_streaming_json, parse_with_repair_value, value_to_jsval, JsVal,
};
use crate::sse::{parse_sse, ServerSentEvent};

const MESSAGE_EVENTS: [&str; 6] = [
	"message_start",
	"message_delta",
	"message_stop",
	"content_block_start",
	"content_block_delta",
	"content_block_stop",
];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
	Text,
	Thinking,
	ToolCall,
}

struct Block {
	index: i64,
	kind: Kind,
	text: String,
	thinking: String,
	thinking_signature: String,
	redacted: bool,
	id: String,
	name: String,
	partial_json: String,
	arguments: JsVal,
}

struct Output {
	content: Vec<Block>,
	api: String,
	provider: String,
	model: String,
	response_id: Option<String>,
	u_input: i64,
	u_output: i64,
	u_cache_read: i64,
	u_cache_write: i64,
	u_cache_write_1h: Option<i64>,
	u_reasoning: Option<i64>,
	u_total: i64,
	stop_reason: String,
	error_message: Option<String>,
}

impl Output {
	fn new(api: &str, provider: &str, model: &str) -> Self {
		Output {
			content: Vec::new(),
			api: api.to_string(),
			provider: provider.to_string(),
			model: model.to_string(),
			response_id: None,
			u_input: 0,
			u_output: 0,
			u_cache_read: 0,
			u_cache_write: 0,
			u_cache_write_1h: None,
			u_reasoning: None,
			u_total: 0,
			stop_reason: "stop".to_string(),
			error_message: None,
		}
	}

	fn find(&self, index: i64) -> Option<usize> {
		self.content.iter().position(|b| b.index == index)
	}

	/// Content array only. Per-event snapshots use this: it is the deterministic, lock-step part of
	/// the stream (text/thinking/tool-argument progression). Message-level terminal fields
	/// (stopReason/errorMessage/usage) are NOT captured per event because, in the TS oracle, a
	/// late-stream mutation (e.g. the error thrown after the last content event) can be observed by
	/// the queued consumer at snapshot time — a timing artifact. Those settle deterministically only
	/// in `final`, which is gated in full.
	fn content_snapshot(&self) -> JsVal {
		JsVal::Arr(
			self.content
				.iter()
				.map(|b| match b.kind {
					Kind::Text => js_obj(vec![("type", js_str("text")), ("text", js_str(&b.text))]),
					Kind::Thinking => {
						let mut pairs = vec![
							("type", js_str("thinking")),
							("thinking", js_str(&b.thinking)),
							("thinkingSignature", js_str(&b.thinking_signature)),
						];
						if b.redacted {
							pairs.push(("redacted", JsVal::Bool(true)));
						}
						js_obj(pairs)
					}
					Kind::ToolCall => js_obj(vec![
						("type", js_str("toolCall")),
						("id", js_str(&b.id)),
						("name", js_str(&b.name)),
						("arguments", b.arguments.clone()),
					]),
				})
				.collect(),
		)
	}

	fn snapshot(&self) -> JsVal {
		let content = self.content_snapshot();

		let mut usage = vec![
			("input", js_num(self.u_input)),
			("output", js_num(self.u_output)),
			("cacheRead", js_num(self.u_cache_read)),
			("cacheWrite", js_num(self.u_cache_write)),
		];
		if let Some(v) = self.u_cache_write_1h {
			usage.push(("cacheWrite1h", js_num(v)));
		}
		if let Some(v) = self.u_reasoning {
			usage.push(("reasoning", js_num(v)));
		}
		usage.push(("totalTokens", js_num(self.u_total)));

		let mut msg = vec![
			("role", js_str("assistant")),
			("api", js_str(&self.api)),
			("provider", js_str(&self.provider)),
			("model", js_str(&self.model)),
		];
		if let Some(id) = &self.response_id {
			msg.push(("responseId", js_str(id)));
		}
		msg.push(("content", content));
		msg.push(("usage", js_obj(usage)));
		msg.push(("stopReason", js_str(&self.stop_reason)));
		if let Some(e) = &self.error_message {
			msg.push(("errorMessage", js_str(e)));
		}
		js_obj(msg)
	}
}

/// Port of mapStopReason. Ok((stopReason, errorMessage)); Err(message) models the JS throw for
/// unhandled reasons (routed to the catch path by the caller).
fn map_stop_reason(reason: &str, stop_details: &Value) -> Result<(String, Option<String>), String> {
	match reason {
		"end_turn" => Ok(("stop".to_string(), None)),
		"max_tokens" => Ok(("length".to_string(), None)),
		"tool_use" => Ok(("toolUse".to_string(), None)),
		"refusal" => {
			let explanation = stop_details["explanation"]
				.as_str()
				.filter(|s| !s.is_empty())
				.unwrap_or("The model refused to complete the request");
			Ok(("error".to_string(), Some(explanation.to_string())))
		}
		"pause_turn" => Ok(("stop".to_string(), None)),
		"stop_sequence" => Ok(("stop".to_string(), None)),
		"sensitive" => Ok(("error".to_string(), None)),
		other => Err(format!("Unhandled stop reason: {other}")),
	}
}

/// Port of iterateAnthropicEvents. Returns the events successfully yielded, plus a terminal error
/// message if the generator threw (event: error / parse failure / ended-before-message_stop).
fn iterate_anthropic_events(sse: &[ServerSentEvent]) -> (Vec<Value>, Option<String>) {
	let mut yielded: Vec<Value> = Vec::new();
	let mut saw_start = false;
	let mut saw_end = false;
	for e in sse {
		if e.event.as_deref() == Some("error") {
			return (yielded, Some(e.data.clone()));
		}
		let name = e.event.clone().unwrap_or_default();
		if !MESSAGE_EVENTS.contains(&name.as_str()) {
			continue;
		}
		match parse_with_repair_value(&e.data) {
			Ok(v) => {
				match v["type"].as_str() {
					Some("message_start") => saw_start = true,
					Some("message_stop") => saw_end = true,
					_ => {}
				}
				yielded.push(v);
			}
			Err(()) => {
				// V8-derived parse error; normalize to a structural sentinel (corpus avoids this path).
				return (yielded, Some(format!("Could not parse Anthropic SSE event {name}")));
			}
		}
	}
	if saw_start && !saw_end {
		return (yielded, Some("Anthropic stream ended before message_stop".to_string()));
	}
	(yielded, None)
}

fn catch(out: &mut Output, events: &mut Vec<JsVal>, message: String) {
	// Signal-abort is not modeled at gate time, so the catch always lands on "error".
	out.stop_reason = "error".to_string();
	out.error_message = Some(message);
	events.push(js_obj(vec![
		("type", js_str("error")),
		("reason", js_str("error")),
		("snapshot", out.content_snapshot()),
	]));
}

/// Decode ordered Anthropic SSE byte chunks into the normalized transcript JsVal.
pub fn decode_anthropic(chunks: &[Vec<u8>], api: &str, provider: &str, model: &str) -> JsVal {
	let sse = parse_sse(chunks);
	let mut out = Output::new(api, provider, model);
	let mut events: Vec<JsVal> = Vec::new();

	// stream.push({ type: "start", partial: output })
	events.push(js_obj(vec![("type", js_str("start")), ("snapshot", out.content_snapshot())]));

	let (yielded, iterate_error) = iterate_anthropic_events(&sse);
	let mut runtime_error: Option<String> = None;

	'assembly: for ev in &yielded {
		match ev["type"].as_str().unwrap_or("") {
			"message_start" => {
				out.response_id = ev["message"]["id"].as_str().map(|s| s.to_string());
				let usage = &ev["message"]["usage"];
				out.u_input = jint(&usage["input_tokens"]);
				out.u_output = jint(&usage["output_tokens"]);
				out.u_cache_read = jint(&usage["cache_read_input_tokens"]);
				out.u_cache_write = jint(&usage["cache_creation_input_tokens"]);
				out.u_cache_write_1h = Some(jint(&usage["cache_creation"]["ephemeral_1h_input_tokens"]));
				out.u_total = out.u_input + out.u_output + out.u_cache_read + out.u_cache_write;
			}
			"content_block_start" => {
				let index = jint(&ev["index"]);
				let cb = &ev["content_block"];
				match cb["type"].as_str().unwrap_or("") {
					"text" => {
						out.content.push(Block {
							index,
							kind: Kind::Text,
							text: String::new(),
							thinking: String::new(),
							thinking_signature: String::new(),
							redacted: false,
							id: String::new(),
							name: String::new(),
							partial_json: String::new(),
							arguments: JsVal::Obj(Vec::new()),
						});
						let ci = out.content.len() - 1;
						events.push(js_obj(vec![
							("type", js_str("text_start")),
							("contentIndex", js_num(ci as i64)),
							("snapshot", out.content_snapshot()),
						]));
					}
					"thinking" => {
						out.content.push(Block {
							index,
							kind: Kind::Thinking,
							text: String::new(),
							thinking: String::new(),
							thinking_signature: String::new(),
							redacted: false,
							id: String::new(),
							name: String::new(),
							partial_json: String::new(),
							arguments: JsVal::Obj(Vec::new()),
						});
						let ci = out.content.len() - 1;
						events.push(js_obj(vec![
							("type", js_str("thinking_start")),
							("contentIndex", js_num(ci as i64)),
							("snapshot", out.content_snapshot()),
						]));
					}
					"redacted_thinking" => {
						out.content.push(Block {
							index,
							kind: Kind::Thinking,
							text: String::new(),
							thinking: "[Reasoning redacted]".to_string(),
							thinking_signature: cb["data"].as_str().unwrap_or("").to_string(),
							redacted: true,
							id: String::new(),
							name: String::new(),
							partial_json: String::new(),
							arguments: JsVal::Obj(Vec::new()),
						});
						let ci = out.content.len() - 1;
						events.push(js_obj(vec![
							("type", js_str("thinking_start")),
							("contentIndex", js_num(ci as i64)),
							("snapshot", out.content_snapshot()),
						]));
					}
					"tool_use" => {
						let input_v = &cb["input"];
						let arguments = if input_v.is_null() {
							JsVal::Obj(Vec::new())
						} else {
							value_to_jsval(input_v)
						};
						out.content.push(Block {
							index,
							kind: Kind::ToolCall,
							text: String::new(),
							thinking: String::new(),
							thinking_signature: String::new(),
							redacted: false,
							id: cb["id"].as_str().unwrap_or("").to_string(),
							name: cb["name"].as_str().unwrap_or("").to_string(),
							partial_json: String::new(),
							arguments,
						});
						let ci = out.content.len() - 1;
						events.push(js_obj(vec![
							("type", js_str("toolcall_start")),
							("contentIndex", js_num(ci as i64)),
							("snapshot", out.content_snapshot()),
						]));
					}
					_ => {}
				}
			}
			"content_block_delta" => {
				let index = jint(&ev["index"]);
				let delta = &ev["delta"];
				let bi = out.find(index);
				match delta["type"].as_str().unwrap_or("") {
					"text_delta" => {
						if let Some(i) = bi {
							if out.content[i].kind == Kind::Text {
								let d = delta["text"].as_str().unwrap_or("").to_string();
								out.content[i].text.push_str(&d);
								events.push(js_obj(vec![
									("type", js_str("text_delta")),
									("contentIndex", js_num(i as i64)),
									("delta", js_str(&d)),
									("snapshot", out.content_snapshot()),
								]));
							}
						}
					}
					"thinking_delta" => {
						if let Some(i) = bi {
							if out.content[i].kind == Kind::Thinking {
								let d = delta["thinking"].as_str().unwrap_or("").to_string();
								out.content[i].thinking.push_str(&d);
								events.push(js_obj(vec![
									("type", js_str("thinking_delta")),
									("contentIndex", js_num(i as i64)),
									("delta", js_str(&d)),
									("snapshot", out.content_snapshot()),
								]));
							}
						}
					}
					"input_json_delta" => {
						if let Some(i) = bi {
							if out.content[i].kind == Kind::ToolCall {
								let d = delta["partial_json"].as_str().unwrap_or("").to_string();
								out.content[i].partial_json.push_str(&d);
								let parsed = parse_streaming_json(&out.content[i].partial_json);
								out.content[i].arguments = parsed;
								events.push(js_obj(vec![
									("type", js_str("toolcall_delta")),
									("contentIndex", js_num(i as i64)),
									("delta", js_str(&d)),
									("snapshot", out.content_snapshot()),
								]));
							}
						}
					}
					"signature_delta" => {
						if let Some(i) = bi {
							if out.content[i].kind == Kind::Thinking {
								let sig = delta["signature"].as_str().unwrap_or("");
								out.content[i].thinking_signature.push_str(sig);
							}
						}
					}
					_ => {}
				}
			}
			"content_block_stop" => {
				let index = jint(&ev["index"]);
				if let Some(i) = out.find(index) {
					match out.content[i].kind {
						Kind::Text => {
							let content = out.content[i].text.clone();
							events.push(js_obj(vec![
								("type", js_str("text_end")),
								("contentIndex", js_num(i as i64)),
								("content", js_str(&content)),
								("snapshot", out.content_snapshot()),
							]));
						}
						Kind::Thinking => {
							let content = out.content[i].thinking.clone();
							events.push(js_obj(vec![
								("type", js_str("thinking_end")),
								("contentIndex", js_num(i as i64)),
								("content", js_str(&content)),
								("snapshot", out.content_snapshot()),
							]));
						}
						Kind::ToolCall => {
							let parsed = parse_streaming_json(&out.content[i].partial_json);
							out.content[i].arguments = parsed;
							events.push(js_obj(vec![
								("type", js_str("toolcall_end")),
								("contentIndex", js_num(i as i64)),
								("snapshot", out.content_snapshot()),
							]));
						}
					}
				}
			}
			"message_delta" => {
				if let Some(reason) = ev["delta"]["stop_reason"].as_str() {
					match map_stop_reason(reason, &ev["delta"]["stop_details"]) {
						Ok((stop, err)) => {
							out.stop_reason = stop;
							if let Some(m) = err {
								out.error_message = Some(m);
							}
						}
						Err(msg) => {
							runtime_error = Some(msg);
							break 'assembly;
						}
					}
				}
				let usage = &ev["usage"];
				if let Some(n) = usage["input_tokens"].as_i64() {
					out.u_input = n;
				}
				if let Some(n) = usage["output_tokens"].as_i64() {
					out.u_output = n;
				}
				if let Some(n) = usage["cache_read_input_tokens"].as_i64() {
					out.u_cache_read = n;
				}
				if let Some(n) = usage["cache_creation_input_tokens"].as_i64() {
					out.u_cache_write = n;
				}
				if let Some(n) = usage["output_tokens_details"]["thinking_tokens"].as_i64() {
					out.u_reasoning = Some(n);
				}
				out.u_total = out.u_input + out.u_output + out.u_cache_read + out.u_cache_write;
			}
			_ => {}
		}
	}

	let terminal = if runtime_error.is_some() { runtime_error } else { iterate_error };
	match terminal {
		Some(msg) => catch(&mut out, &mut events, msg),
		None => {
			if out.stop_reason == "error" || out.stop_reason == "aborted" {
				let msg = out
					.error_message
					.clone()
					.unwrap_or_else(|| "An unknown error occurred".to_string());
				catch(&mut out, &mut events, msg);
			} else {
				events.push(js_obj(vec![
					("type", js_str("done")),
					("reason", js_str(&out.stop_reason)),
					("snapshot", out.content_snapshot()),
				]));
			}
		}
	}

	js_obj(vec![("events", JsVal::Arr(events)), ("final", out.snapshot())])
}
