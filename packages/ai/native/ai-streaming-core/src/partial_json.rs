//! Faithful port of packages/ai/src/utils/json-parse.ts and its `partial-json` 0.1.7 dependency.
//!
//! Fidelity rules (load-bearing, confirmed by reading the TS + the installed partial-json source):
//! - All string indexing mirrors JS UTF-16 code units (`json[i]`, `.length`, `.slice`, `.substring`).
//!   We operate on `Vec<u16>`, not `char`s/bytes, so astral chars and \u handling match JS arithmetic.
//! - `parse` (partial-json) = `parseJSON` with default `Allow.ALL` (every partial type permitted).
//! - The streaming ladder's `?? {}` only coalesces JS null/undefined: 0/false/""/NaN/Infinity SURVIVE.
//!   We model NaN/±Infinity distinctly from Null so the `??` step matches, then canonicalize them to
//!   `null` (as `JSON.stringify` does) only at serialization time.
//! - Leaf `JSON.parse(...)` calls are delegated to serde_json (the JS-JSON.parse equivalent for our inputs).

use serde_json::Value;

const QUOTE: u16 = b'"' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const SLASH: u16 = b'/' as u16;
const LBRACE: u16 = b'{' as u16;
const RBRACE: u16 = b'}' as u16;
const LBRACK: u16 = b'[' as u16;
const RBRACK: u16 = b']' as u16;
const COMMA: u16 = b',' as u16;
const MINUS: u16 = b'-' as u16;
const U_LOWER: u16 = b'u' as u16;
const E_LOWER: u16 = b'e' as u16;

/// JS value model that preserves the distinctions that survive `?? {}`.
#[derive(Debug, Clone)]
pub enum JsVal {
	Null,
	Nan,
	Inf,
	NegInf,
	Bool(bool),
	Num(f64),
	Str(String),
	Arr(Vec<JsVal>),
	Obj(Vec<(String, JsVal)>),
}

// ── Shared JsVal builders ─────────────────────────────────────────────────────────────────────────
// Lifted here (from anthropic.rs) so every decoder builds snapshots against the same canonical contract.

pub fn js_str(x: &str) -> JsVal {
	JsVal::Str(x.to_string())
}

pub fn js_num(x: i64) -> JsVal {
	JsVal::Num(x as f64)
}

pub fn js_obj(pairs: Vec<(&str, JsVal)>) -> JsVal {
	JsVal::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// Read an integer from a serde_json number, tolerating float-encoded integers; non-numbers -> 0.
pub fn jint(v: &Value) -> i64 {
	v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).unwrap_or(0)
}

fn is_valid_json_escape(u: u16) -> bool {
	// VALID_JSON_ESCAPES = {", \, /, b, f, n, r, t, u}
	matches!(
		u,
		QUOTE | BACKSLASH | SLASH | 0x62 /*b*/ | 0x66 /*f*/ | 0x6E /*n*/ | 0x72 /*r*/ | 0x74 /*t*/ | U_LOWER
	)
}

fn is_hex(u: u16) -> bool {
	matches!(u, 0x30..=0x39 | 0x41..=0x46 | 0x61..=0x66)
}

fn is_control(u: u16) -> bool {
	u <= 0x1f
}

fn escape_control(u: u16) -> Vec<u16> {
	let s: String = match u {
		0x08 => "\\b".into(),
		0x0c => "\\f".into(),
		0x0a => "\\n".into(),
		0x0d => "\\r".into(),
		0x09 => "\\t".into(),
		_ => format!("\\u{:04x}", u),
	};
	s.encode_utf16().collect()
}

/// Port of repairJson (json-parse.ts:32-83), over UTF-16 units.
fn repair_units(json: &[u16]) -> Vec<u16> {
	let mut out: Vec<u16> = Vec::with_capacity(json.len());
	let mut in_string = false;
	let n = json.len();
	let mut index = 0usize;
	while index < n {
		let ch = json[index];

		if !in_string {
			out.push(ch);
			if ch == QUOTE {
				in_string = true;
			}
			index += 1;
			continue;
		}

		if ch == QUOTE {
			out.push(ch);
			in_string = false;
			index += 1;
			continue;
		}

		if ch == BACKSLASH {
			match json.get(index + 1).copied() {
				None => {
					out.push(BACKSLASH);
					out.push(BACKSLASH);
					index += 1; // loop-equivalent advance only (matches JS `continue`)
					continue;
				}
				Some(next) => {
					if next == U_LOWER && index + 6 <= n {
						let digits = &json[index + 2..index + 6];
						if digits.iter().all(|&d| is_hex(d)) {
							out.push(BACKSLASH);
							out.push(U_LOWER);
							out.extend_from_slice(digits);
							index += 6; // JS: index += 5 then loop index++
							continue;
						}
					}
					if is_valid_json_escape(next) {
						out.push(BACKSLASH);
						out.push(next);
						index += 2; // JS: index += 1 then loop index++
						continue;
					}
					out.push(BACKSLASH);
					out.push(BACKSLASH);
					index += 1; // nextChar reprocessed next iteration
					continue;
				}
			}
		}

		if is_control(ch) {
			out.extend(escape_control(ch));
		} else {
			out.push(ch);
		}
		index += 1;
	}
	out
}

pub fn repair_string(s: &str) -> String {
	let units: Vec<u16> = s.encode_utf16().collect();
	String::from_utf16_lossy(&repair_units(&units))
}

/// Port of parseJsonWithRepair returning a serde_json::Value, for navigating SSE event payloads.
/// Mirrors json-parse.ts: JSON.parse, and on failure repairJson + JSON.parse (else rethrow).
pub fn parse_with_repair_value(json: &str) -> Result<Value, ()> {
	match serde_json::from_str::<Value>(json) {
		Ok(v) => Ok(v),
		Err(_) => {
			let repaired = repair_string(json);
			if repaired != json {
				serde_json::from_str::<Value>(&repaired).map_err(|_| ())
			} else {
				Err(())
			}
		}
	}
}

/// JS `JSON.parse` equivalent for our inputs.
fn json_parse_str(s: &str) -> Result<JsVal, ()> {
	let v: Value = serde_json::from_str(s).map_err(|_| ())?;
	Ok(value_to_jsval(&v))
}

pub fn value_to_jsval(v: &Value) -> JsVal {
	match v {
		Value::Null => JsVal::Null,
		Value::Bool(b) => JsVal::Bool(*b),
		Value::Number(n) => JsVal::Num(n.as_f64().unwrap_or(0.0)),
		Value::String(s) => JsVal::Str(s.clone()),
		Value::Array(a) => JsVal::Arr(a.iter().map(value_to_jsval).collect()),
		Value::Object(m) => JsVal::Obj(m.iter().map(|(k, v)| (k.clone(), value_to_jsval(v))).collect()),
	}
}

/// Port of partial-json 0.1.7 `_parseJSON` with `Allow.ALL`.
struct Parser {
	u: Vec<u16>,
	index: usize,
}

impl Parser {
	fn len(&self) -> usize {
		self.u.len()
	}

	fn at(&self, i: usize) -> Option<u16> {
		self.u.get(i).copied()
	}

	fn substr_string(&self, a: usize, b: usize) -> String {
		// Mirror String.prototype.substring: clamp to [0,len] and swap if a>b.
		let len = self.len();
		let a = a.min(len);
		let b = b.min(len);
		let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
		String::from_utf16_lossy(&self.u[lo..hi])
	}

	fn sub_eq(&self, a: usize, b: usize, lit: &str) -> bool {
		let len = self.len();
		let a = a.min(len);
		let b = b.min(len);
		if a > b {
			return false;
		}
		let lit_units: Vec<u16> = lit.encode_utf16().collect();
		self.u[a..b] == lit_units[..]
	}

	fn last_index_of(&self, unit: u16) -> Option<usize> {
		self.u.iter().rposition(|&c| c == unit)
	}

	fn skip_blank(&mut self) {
		while self.index < self.len() {
			match self.u[self.index] {
				0x20 | 0x0a | 0x0d | 0x09 => self.index += 1,
				_ => break,
			}
		}
	}

	fn parse_any(&mut self) -> Result<JsVal, ()> {
		self.skip_blank();
		if self.index >= self.len() {
			return Err(()); // markPartialJSON("Unexpected end of input")
		}
		let c = self.u[self.index];
		if c == QUOTE {
			return self.parse_str();
		}
		if c == LBRACE {
			return self.parse_obj();
		}
		if c == LBRACK {
			return self.parse_arr();
		}
		let rem = self.len() - self.index;
		// null
		if self.sub_eq(self.index, self.index + 4, "null")
			|| (rem < 4 && "null".starts_with(&self.substr_string(self.index, self.len())))
		{
			self.index += 4;
			return Ok(JsVal::Null);
		}
		// true
		if self.sub_eq(self.index, self.index + 4, "true")
			|| (rem < 4 && "true".starts_with(&self.substr_string(self.index, self.len())))
		{
			self.index += 4;
			return Ok(JsVal::Bool(true));
		}
		// false
		if self.sub_eq(self.index, self.index + 5, "false")
			|| (rem < 5 && "false".starts_with(&self.substr_string(self.index, self.len())))
		{
			self.index += 5;
			return Ok(JsVal::Bool(false));
		}
		// Infinity
		if self.sub_eq(self.index, self.index + 8, "Infinity")
			|| (rem < 8 && "Infinity".starts_with(&self.substr_string(self.index, self.len())))
		{
			self.index += 8;
			return Ok(JsVal::Inf);
		}
		// -Infinity
		if self.sub_eq(self.index, self.index + 9, "-Infinity")
			|| (1 < rem && rem < 9 && "-Infinity".starts_with(&self.substr_string(self.index, self.len())))
		{
			self.index += 9;
			return Ok(JsVal::NegInf);
		}
		// NaN
		if self.sub_eq(self.index, self.index + 3, "NaN")
			|| (rem < 3 && "NaN".starts_with(&self.substr_string(self.index, self.len())))
		{
			self.index += 3;
			return Ok(JsVal::Nan);
		}
		self.parse_num()
	}

	fn parse_str(&mut self) -> Result<JsVal, ()> {
		let start = self.index;
		let mut escape = false;
		self.index += 1; // skip initial quote
		while self.index < self.len()
			&& (self.u[self.index] != QUOTE || (escape && self.u[self.index - 1] == BACKSLASH))
		{
			escape = if self.u[self.index] == BACKSLASH { !escape } else { false };
			self.index += 1;
		}
		if self.at(self.index) == Some(QUOTE) {
			self.index += 1;
			let end = self.index - (escape as usize);
			let s = self.substr_string(start, end);
			return json_parse_str(&s); // on err -> Err (throwMalformedError)
		}
		// else if Allow.STR & allow (always true with Allow.ALL)
		let end = self.index - (escape as usize);
		let mut first = self.substr_string(start, end);
		first.push('"');
		match json_parse_str(&first) {
			Ok(v) => Ok(v),
			Err(()) => {
				let li = self.last_index_of(BACKSLASH);
				// JS: substring(start, lastIndexOf("\\")); lastIndexOf -> -1 maps to substring(start, 0)
				let end2 = li.unwrap_or(0);
				let mut second = self.substr_string(start, end2);
				second.push('"');
				json_parse_str(&second)
			}
		}
	}

	fn parse_obj(&mut self) -> Result<JsVal, ()> {
		self.index += 1; // skip {
		self.skip_blank();
		let mut obj: Vec<(String, JsVal)> = Vec::new();
		let inner = (|| -> Result<(), ()> {
			while self.at(self.index) != Some(RBRACE) {
				self.skip_blank();
				if self.index >= self.len() {
					return Ok(()); // Allow.OBJ -> return obj (early)
				}
				let key = match self.parse_str()? {
					JsVal::Str(s) => s,
					// parse_str always yields a string here for well-formed keys; other shapes
					// come from JSON.parse of the key literal and are coerced to their JSON text.
					other => canonical(&other),
				};
				self.skip_blank();
				self.index += 1; // skip colon (blind, as in JS)
				match self.parse_any() {
					Ok(value) => set_key(&mut obj, key, value),
					Err(()) => return Ok(()), // Allow.OBJ -> return obj
				}
				self.skip_blank();
				if self.at(self.index) == Some(COMMA) {
					self.index += 1; // skip comma
				}
			}
			Ok(())
		})();
		// JS: if the loop body / key parse throws, the outer catch returns obj (Allow.OBJ).
		let _ = inner;
		self.index += 1; // skip }
		Ok(JsVal::Obj(obj))
	}

	fn parse_arr(&mut self) -> Result<JsVal, ()> {
		self.index += 1; // skip [
		let mut arr: Vec<JsVal> = Vec::new();
		let _ = (|| -> Result<(), ()> {
			while self.at(self.index) != Some(RBRACK) {
				arr.push(self.parse_any()?);
				self.skip_blank();
				if self.at(self.index) == Some(COMMA) {
					self.index += 1;
				}
			}
			Ok(())
		})();
		// JS: catch -> Allow.ARR -> return arr.
		self.index += 1; // skip ]
		Ok(JsVal::Arr(arr))
	}

	fn parse_num(&mut self) -> Result<JsVal, ()> {
		if self.index == 0 {
			if self.u == "-".encode_utf16().collect::<Vec<_>>() {
				return Err(());
			}
			match json_parse_str(&self.substr_string(0, self.len())) {
				Ok(v) => return Ok(v),
				Err(()) => {
					// NUM allowed: try JSON.parse(substring(0, lastIndexOf("e")))
					if let Some(ei) = self.last_index_of(E_LOWER) {
						if let Ok(v) = json_parse_str(&self.substr_string(0, ei)) {
							return Ok(v);
						}
					}
					return Err(());
				}
			}
		}
		let start = self.index;
		if self.at(self.index) == Some(MINUS) {
			self.index += 1;
		}
		while self.index < self.len() {
			let ch = self.u[self.index];
			if ch == COMMA || ch == RBRACK || ch == RBRACE {
				break;
			}
			self.index += 1;
		}
		// if (index == length && !(NUM & allow)) markPartial -> NUM allowed, skip
		match json_parse_str(&self.substr_string(start, self.index)) {
			Ok(v) => Ok(v),
			Err(()) => {
				if self.substr_string(start, self.index) == "-" {
					return Err(());
				}
				if let Some(ei) = self.last_index_of(E_LOWER) {
					return json_parse_str(&self.substr_string(start, ei));
				}
				Err(())
			}
		}
	}
}

fn set_key(obj: &mut Vec<(String, JsVal)>, key: String, value: JsVal) {
	// JS object assignment: last write wins for duplicate keys.
	if let Some(slot) = obj.iter_mut().find(|(k, _)| *k == key) {
		slot.1 = value;
	} else {
		obj.push((key, value));
	}
}

fn partial_parse(input: &str) -> Result<JsVal, ()> {
	let trimmed = input.trim();
	if trimmed.is_empty() {
		return Err(()); // parseJSON throws on empty
	}
	let mut p = Parser {
		u: trimmed.encode_utf16().collect(),
		index: 0,
	};
	p.parse_any()
}

/// Port of parseJsonWithRepair (json-parse.ts:85-95).
fn parse_json_with_repair(json: &str) -> Result<JsVal, ()> {
	match json_parse_str(json) {
		Ok(v) => Ok(v),
		Err(()) => {
			let repaired = repair_string(json);
			if repaired != json {
				json_parse_str(&repaired)
			} else {
				Err(())
			}
		}
	}
}

fn coalesce_null(v: JsVal) -> JsVal {
	// JS `result ?? {}`: only null/undefined coalesce to {}.
	match v {
		JsVal::Null => JsVal::Obj(Vec::new()),
		other => other,
	}
}

/// Port of parseStreamingJson (json-parse.ts:104-124).
pub fn parse_streaming_json(input: &str) -> JsVal {
	if input.trim().is_empty() {
		return JsVal::Obj(Vec::new());
	}
	if let Ok(v) = parse_json_with_repair(input) {
		return v; // fast path: NO coalescing
	}
	if let Ok(v) = partial_parse(input) {
		return coalesce_null(v);
	}
	if let Ok(v) = partial_parse(&repair_string(input)) {
		return coalesce_null(v);
	}
	JsVal::Obj(Vec::new())
}

// ---- Canonical serialization (must match the TS sortKeys + JSON.stringify oracle) ----

fn format_num(n: f64) -> String {
	if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
		format!("{}", n as i64)
	} else {
		format!("{}", n)
	}
}

fn escape_json_string(s: &str) -> String {
	let mut out = String::with_capacity(s.len() + 2);
	out.push('"');
	for c in s.chars() {
		match c {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\u{08}' => out.push_str("\\b"),
			'\u{0c}' => out.push_str("\\f"),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
			c => out.push(c),
		}
	}
	out.push('"');
	out
}

/// Canonical JSON string matching the TS oracle: sorted object keys, NaN/Infinity -> null.
pub fn canonical(v: &JsVal) -> String {
	match v {
		JsVal::Null | JsVal::Nan | JsVal::Inf | JsVal::NegInf => "null".to_string(),
		JsVal::Bool(true) => "true".to_string(),
		JsVal::Bool(false) => "false".to_string(),
		JsVal::Num(n) => format_num(*n),
		JsVal::Str(s) => escape_json_string(s),
		JsVal::Arr(items) => {
			let inner: Vec<String> = items.iter().map(canonical).collect();
			format!("[{}]", inner.join(","))
		}
		JsVal::Obj(entries) => {
			let mut dedup: Vec<(String, &JsVal)> = Vec::new();
			for (k, val) in entries {
				if let Some(slot) = dedup.iter_mut().find(|(ek, _)| ek == k) {
					slot.1 = val;
				} else {
					dedup.push((k.clone(), val));
				}
			}
			dedup.sort_by(|a, b| a.0.cmp(&b.0));
			let inner: Vec<String> = dedup
				.iter()
				.map(|(k, val)| format!("{}:{}", escape_json_string(k), canonical(val)))
				.collect();
			format!("{{{}}}", inner.join(","))
		}
	}
}
