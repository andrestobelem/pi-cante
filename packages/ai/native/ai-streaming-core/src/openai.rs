//! Faithful port of openai-completions.ts: the stream() assembly loop (L316-471), parseChunkUsage
//! (L1104-1140), and mapStopReason (L1142-1166). Produces a normalized transcript (the ordered
//! AssistantMessageEvent sequence, plus the fully-settled final message) as a JsVal, ready for canonical
//! serialization against the TS oracle.
//!
//! Unlike the Anthropic decoder, the OpenAI SDK decodes SSE internally, so this operates on already-parsed
//! ChatCompletionChunk objects (`&[serde_json::Value]`) — there is no SSE framing. OpenAI and Ollama share
//! this exact path; the decoder never branches on provider.
//!
//! PER-EVENT SNAPSHOTS ARE NOT GATED (this is the one deliberate departure from the Anthropic gate).
//! Every TS event carries `partial: output` aliasing the same mutable object. The EventStream either
//! hand-delivers an event to a waiting consumer (resolving a promise — a microtask hop) or queues it, and
//! its async iterator drains the queue via `yield queue.shift()` with NO await (utils/event-stream.ts:31,
//! 52-53). Either way the synchronous producer races ahead mutating `output` between the consumer's
//! microtask-spaced reads, so a per-event snapshot reflects a content state AHEAD of synchronous emission —
//! deterministic per TS run, but a consumer-drain timing artifact a synchronous decoder cannot reproduce.
//! (The Anthropic producer awaits between events, so its snapshots happen to be progressive; OpenAI's are
//! not.) We instead gate the deterministic, reproducible contract: the event SEQUENCE (type, contentIndex,
//! delta, content, reason) and the fully-settled `final` message — which together pin every assembly-loop
//! behavior (block-creation order, contentIndex assignment, signature-lock, batched *_end ordering,
//! error-vs-done termination, and the final parsed tool arguments).
//!
//! Fidelity notes (load-bearing):
//! - Tool-call arguments accumulate as a string and are re-parsed at finalize through `parse_streaming_json`
//!   (the same Increment-1 entrypoint the TS oracle imports). Never a serde_json passthrough — that would
//!   diverge on numeric/NaN/Infinity canonicalization.
//! - thinkingSignature is locked at thinking-block creation; later reasoning deltas never re-stamp it.
//! - Block-creation order within one delta is content -> reasoning -> tool_calls, and contentIndex is the
//!   insertion position; the dual streamIndex/id maps mirror toolCallBlocksByIndex/ById with lazy back-fill.
//! - All `*_end` events are emitted in a single post-loop pass over content (NOT interleaved like Anthropic).
//! - The encrypted `reasoning_details` / thoughtSignature path is intentionally OUT OF SCOPE this increment.

use std::collections::HashMap;

use serde_json::Value;

use crate::partial_json::{jint, js_num, js_obj, js_str, parse_streaming_json, JsVal};

#[derive(Clone, Copy, PartialEq)]
enum OKind {
	Text,
	Thinking,
	ToolCall,
}

struct OBlock {
	kind: OKind,
	text: String,
	thinking: String,
	thinking_signature: String,
	id: String,
	name: String,
	partial_args: String,
	stream_index: Option<i64>,
	arguments: JsVal,
}

impl OBlock {
	fn text() -> Self {
		OBlock {
			kind: OKind::Text,
			text: String::new(),
			thinking: String::new(),
			thinking_signature: String::new(),
			id: String::new(),
			name: String::new(),
			partial_args: String::new(),
			stream_index: None,
			arguments: JsVal::Obj(Vec::new()),
		}
	}

	fn thinking(signature: &str) -> Self {
		OBlock {
			kind: OKind::Thinking,
			text: String::new(),
			thinking: String::new(),
			thinking_signature: signature.to_string(),
			id: String::new(),
			name: String::new(),
			partial_args: String::new(),
			stream_index: None,
			arguments: JsVal::Obj(Vec::new()),
		}
	}

	fn tool(id: &str, name: &str, stream_index: Option<i64>) -> Self {
		OBlock {
			kind: OKind::ToolCall,
			text: String::new(),
			thinking: String::new(),
			thinking_signature: String::new(),
			id: id.to_string(),
			name: name.to_string(),
			partial_args: String::new(),
			stream_index,
			arguments: JsVal::Obj(Vec::new()),
		}
	}
}

struct OOut {
	content: Vec<OBlock>,
	api: String,
	provider: String,
	model: String,
	response_id: Option<String>,
	response_model: Option<String>,
	u_input: i64,
	u_output: i64,
	u_cache_read: i64,
	u_cache_write: i64,
	u_reasoning: Option<i64>,
	u_total: i64,
	stop_reason: String,
	error_message: Option<String>,
}

impl OOut {
	fn new(api: &str, provider: &str, model: &str) -> Self {
		OOut {
			content: Vec::new(),
			api: api.to_string(),
			provider: provider.to_string(),
			model: model.to_string(),
			response_id: None,
			response_model: None,
			u_input: 0,
			u_output: 0,
			u_cache_read: 0,
			u_cache_write: 0,
			u_reasoning: None,
			u_total: 0,
			stop_reason: "stop".to_string(),
			error_message: None,
		}
	}

	/// The fully-settled final message (content with parsed tool arguments, usage, stopReason, etc.).
	fn snapshot(&self) -> JsVal {
		let content = JsVal::Arr(
			self.content
				.iter()
				.map(|b| match b.kind {
					OKind::Text => js_obj(vec![("type", js_str("text")), ("text", js_str(&b.text))]),
					OKind::Thinking => js_obj(vec![
						("type", js_str("thinking")),
						("thinking", js_str(&b.thinking)),
						("thinkingSignature", js_str(&b.thinking_signature)),
					]),
					OKind::ToolCall => js_obj(vec![
						("type", js_str("toolCall")),
						("id", js_str(&b.id)),
						("name", js_str(&b.name)),
						("arguments", b.arguments.clone()),
					]),
				})
				.collect(),
		);

		let mut usage = vec![
			("input", js_num(self.u_input)),
			("output", js_num(self.u_output)),
			("cacheRead", js_num(self.u_cache_read)),
			("cacheWrite", js_num(self.u_cache_write)),
		];
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
		if let Some(m) = &self.response_model {
			msg.push(("responseModel", js_str(m)));
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

/// Port of mapStopReason. Returns (stopReason, errorMessage); never throws (the JS `null` branch is
/// unreachable here because the caller guards on a truthy finish_reason string).
fn map_stop_reason(reason: &str) -> (String, Option<String>) {
	match reason {
		"stop" | "end" => ("stop".to_string(), None),
		"length" => ("length".to_string(), None),
		"function_call" | "tool_calls" => ("toolUse".to_string(), None),
		"content_filter" => ("error".to_string(), Some("Provider finish_reason: content_filter".to_string())),
		"network_error" => ("error".to_string(), Some("Provider finish_reason: network_error".to_string())),
		other => ("error".to_string(), Some(format!("Provider finish_reason: {other}"))),
	}
}

/// Port of parseChunkUsage. Replicates the `||`-vs-`??` coalescing exactly: a literal 0 for cached_tokens
/// short-circuits the `??` chain (does NOT fall through to prompt_cache_hit_tokens).
fn apply_usage(out: &mut OOut, raw: &Value) {
	// Token counts are read float-tolerantly via `jint` (which accepts float-encoded integers like 100.0),
	// matching the TS oracle's `x || 0` where JS treats 100.0 as 100. Plain `as_i64()` returns None for a
	// float-typed Number, which would diverge on a provider that wire-encodes counts as floats. The
	// cache-read `??` chain uses `as_f64()` for PRESENCE (a present 0 short-circuits — does NOT fall through
	// to prompt_cache_hit_tokens; absent/null falls through). (No fixture locks the float case: JSON.stringify
	// serializes integer-valued floats back to integers, so the golden cannot represent "100.0"; this is a
	// defensive real-wire faithfulness fix consistent with the codebase's `jint`.)
	let prompt = jint(&raw["prompt_tokens"]);
	let cache_read = if let Some(n) = raw["prompt_tokens_details"]["cached_tokens"].as_f64() {
		n as i64
	} else if let Some(n) = raw["prompt_cache_hit_tokens"].as_f64() {
		n as i64
	} else {
		0
	};
	let cache_write = jint(&raw["prompt_tokens_details"]["cache_write_tokens"]);
	let input = (prompt - cache_read - cache_write).max(0);
	let output = jint(&raw["completion_tokens"]);
	let reasoning = jint(&raw["completion_tokens_details"]["reasoning_tokens"]);
	out.u_input = input;
	out.u_output = output;
	out.u_cache_read = cache_read;
	out.u_cache_write = cache_write;
	out.u_reasoning = Some(reasoning);
	out.u_total = input + output + cache_read + cache_write;
}

fn catch(out: &mut OOut, events: &mut Vec<JsVal>, message: String) {
	out.stop_reason = "error".to_string();
	out.error_message = Some(message);
	events.push(js_obj(vec![("type", js_str("error")), ("reason", js_str("error"))]));
}

fn ensure_text_block(out: &mut OOut, text_idx: &mut Option<usize>, events: &mut Vec<JsVal>) -> usize {
	if let Some(i) = *text_idx {
		return i;
	}
	out.content.push(OBlock::text());
	let i = out.content.len() - 1;
	*text_idx = Some(i);
	events.push(js_obj(vec![("type", js_str("text_start")), ("contentIndex", js_num(i as i64))]));
	i
}

fn ensure_thinking_block(out: &mut OOut, thinking_idx: &mut Option<usize>, signature: &str, events: &mut Vec<JsVal>) -> usize {
	if let Some(i) = *thinking_idx {
		return i;
	}
	out.content.push(OBlock::thinking(signature));
	let i = out.content.len() - 1;
	*thinking_idx = Some(i);
	events.push(js_obj(vec![("type", js_str("thinking_start")), ("contentIndex", js_num(i as i64))]));
	i
}

/// Port of ensureToolCallBlock: streamIndex-first, id-fallback lookup with lazy back-fill of both maps.
/// (applyPendingReasoningDetail is omitted — the encrypted reasoning path is deferred.)
fn ensure_tool_call_block(
	out: &mut OOut,
	by_index: &mut HashMap<i64, usize>,
	by_id: &mut HashMap<String, usize>,
	events: &mut Vec<JsVal>,
	tc: &Value,
) -> usize {
	let stream_index = tc["index"].as_i64();
	let tc_id = tc["id"].as_str().filter(|s| !s.is_empty());

	let mut pos: Option<usize> = stream_index.and_then(|si| by_index.get(&si).copied());
	if pos.is_none() {
		if let Some(id) = tc_id {
			pos = by_id.get(id).copied();
		}
	}
	if pos.is_none() {
		let id0 = tc_id.unwrap_or("");
		let name0 = tc["function"]["name"].as_str().filter(|s| !s.is_empty()).unwrap_or("");
		out.content.push(OBlock::tool(id0, name0, stream_index));
		let p = out.content.len() - 1;
		if let Some(si) = stream_index {
			by_index.insert(si, p);
		}
		if let Some(id) = tc_id {
			by_id.insert(id.to_string(), p);
		}
		events.push(js_obj(vec![("type", js_str("toolcall_start")), ("contentIndex", js_num(p as i64))]));
		pos = Some(p);
	}
	let p = pos.unwrap();
	if let Some(si) = stream_index {
		if out.content[p].stream_index.is_none() {
			out.content[p].stream_index = Some(si);
			by_index.insert(si, p);
		}
	}
	if let Some(id) = tc_id {
		by_id.insert(id.to_string(), p);
	}
	p
}

/// Decode an ordered array of already-parsed ChatCompletionChunk objects into the normalized transcript.
pub fn decode_openai(chunks: &[Value], api: &str, provider: &str, model: &str) -> JsVal {
	let mut out = OOut::new(api, provider, model);
	let mut events: Vec<JsVal> = Vec::new();

	// stream.push({ type: "start", partial: output })
	events.push(js_obj(vec![("type", js_str("start"))]));

	let mut text_idx: Option<usize> = None;
	let mut thinking_idx: Option<usize> = None;
	let mut by_index: HashMap<i64, usize> = HashMap::new();
	let mut by_id: HashMap<String, usize> = HashMap::new();
	let mut has_finish_reason = false;

	for chunk in chunks {
		// if (!chunk || typeof chunk !== "object") continue;
		if !chunk.is_object() {
			continue;
		}

		// output.responseId ||= chunk.id  (first-truthy-wins, falsy skipped)
		let rid_falsy = match &out.response_id {
			None => true,
			Some(s) => s.is_empty(),
		};
		if rid_falsy {
			out.response_id = chunk["id"].as_str().map(|s| s.to_string());
		}

		// responseModel ||= chunk.model  (only when a non-empty string differing from model.id)
		if let Some(cm) = chunk["model"].as_str() {
			if !cm.is_empty() && cm != out.model {
				let rm_falsy = match &out.response_model {
					None => true,
					Some(s) => s.is_empty(),
				};
				if rm_falsy {
					out.response_model = Some(cm.to_string());
				}
			}
		}

		// if (chunk.usage) output.usage = parseChunkUsage(chunk.usage)
		if chunk["usage"].is_object() {
			apply_usage(&mut out, &chunk["usage"]);
		}

		// const choice = Array.isArray(chunk.choices) ? chunk.choices[0] : undefined; if (!choice) continue;
		let choice = match chunk["choices"].as_array().and_then(|a| a.first()) {
			Some(c) if !c.is_null() => c,
			_ => continue,
		};

		// Moonshot fallback: usage in choice.usage when chunk.usage is absent.
		if !chunk["usage"].is_object() && choice["usage"].is_object() {
			apply_usage(&mut out, &choice["usage"]);
		}

		// if (choice.finish_reason) { ... }
		if let Some(reason) = choice["finish_reason"].as_str().filter(|s| !s.is_empty()) {
			let (stop, err) = map_stop_reason(reason);
			out.stop_reason = stop;
			if let Some(m) = err {
				out.error_message = Some(m);
			}
			has_finish_reason = true;
		}

		if !choice["delta"].is_object() {
			continue;
		}
		let delta = &choice["delta"];

		// content (skip null/undefined/empty)
		if let Some(c) = delta["content"].as_str() {
			if !c.is_empty() {
				let i = ensure_text_block(&mut out, &mut text_idx, &mut events);
				out.content[i].text.push_str(c);
				events.push(js_obj(vec![
					("type", js_str("text_delta")),
					("contentIndex", js_num(i as i64)),
					("delta", js_str(c)),
				]));
			}
		}

		// reasoning: first non-empty of [reasoning_content, reasoning, reasoning_text]; signature = field name.
		let mut found: Option<(&str, String)> = None;
		for field in ["reasoning_content", "reasoning", "reasoning_text"] {
			if let Some(v) = delta[field].as_str() {
				if !v.is_empty() {
					found = Some((field, v.to_string()));
					break;
				}
			}
		}
		if let Some((field, val)) = found {
			// model.provider is openai/ollama at gate time, so the opencode-go signature override never fires.
			let i = ensure_thinking_block(&mut out, &mut thinking_idx, field, &mut events);
			out.content[i].thinking.push_str(&val);
			events.push(js_obj(vec![
				("type", js_str("thinking_delta")),
				("contentIndex", js_num(i as i64)),
				("delta", js_str(&val)),
			]));
		}

		// tool_calls
		if let Some(arr) = delta["tool_calls"].as_array() {
			for tc in arr {
				let pos = ensure_tool_call_block(&mut out, &mut by_index, &mut by_id, &mut events, tc);
				// Late id back-fill (fill-once); does NOT re-apply pending reasoning details.
				let tc_id = tc["id"].as_str().filter(|s| !s.is_empty());
				if out.content[pos].id.is_empty() {
					if let Some(id) = tc_id {
						out.content[pos].id = id.to_string();
						by_id.insert(id.to_string(), pos);
					}
				}
				if out.content[pos].name.is_empty() {
					if let Some(n) = tc["function"]["name"].as_str().filter(|s| !s.is_empty()) {
						out.content[pos].name = n.to_string();
					}
				}
				// JS truthiness: an empty-string arguments fragment is falsy -> no concat, no reparse, delta "".
				let mut d = String::new();
				if let Some(args) = tc["function"]["arguments"].as_str() {
					if !args.is_empty() {
						d = args.to_string();
						out.content[pos].partial_args.push_str(args);
						let parsed = parse_streaming_json(&out.content[pos].partial_args);
						out.content[pos].arguments = parsed;
					}
				}
				events.push(js_obj(vec![
					("type", js_str("toolcall_delta")),
					("contentIndex", js_num(pos as i64)),
					("delta", js_str(&d)),
				]));
			}
		}

		// reasoning_details (encrypted thoughtSignature): DEFERRED — intentionally not modeled this increment.
	}

	// Post-loop finishBlock pass over content in insertion order (all *_end events batch here).
	for i in 0..out.content.len() {
		match out.content[i].kind {
			OKind::Text => {
				let content = out.content[i].text.clone();
				events.push(js_obj(vec![
					("type", js_str("text_end")),
					("contentIndex", js_num(i as i64)),
					("content", js_str(&content)),
				]));
			}
			OKind::Thinking => {
				let content = out.content[i].thinking.clone();
				events.push(js_obj(vec![
					("type", js_str("thinking_end")),
					("contentIndex", js_num(i as i64)),
					("content", js_str(&content)),
				]));
			}
			OKind::ToolCall => {
				let parsed = parse_streaming_json(&out.content[i].partial_args);
				out.content[i].arguments = parsed;
				events.push(js_obj(vec![
					("type", js_str("toolcall_end")),
					("contentIndex", js_num(i as i64)),
				]));
			}
		}
	}

	// Terminal: error stop reason throws (errorMessage || fallback), then no-finish_reason throws; else done.
	// (The `aborted` branch is unreachable at gate time: mapStopReason never yields it and no signal is set.)
	let terminal: Option<String> = if out.stop_reason == "error" {
		Some(
			out.error_message
				.clone()
				.unwrap_or_else(|| "Provider returned an error stop reason".to_string()),
		)
	} else if !has_finish_reason {
		Some("Stream ended without finish_reason".to_string())
	} else {
		None
	};

	match terminal {
		Some(msg) => catch(&mut out, &mut events, msg),
		None => {
			events.push(js_obj(vec![("type", js_str("done")), ("reason", js_str(&out.stop_reason))]));
		}
	}

	js_obj(vec![("events", JsVal::Arr(events)), ("final", out.snapshot())])
}
