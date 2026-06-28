//! Differential conformance: decode_anthropic must reproduce, byte-for-byte, the canonical transcript
//! captured from the TS oracle in packages/ai/test/gate/fixtures/anthropic.golden.json, from the same
//! recorded SSE byte chunks.

use ai_streaming_core::{canonical, decode_anthropic};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Deserialize)]
struct Row {
	name: String,
	api: String,
	provider: String,
	model: String,
	chunks: Vec<Vec<u8>>,
	expected: String,
}

#[test]
fn anthropic_decoder_parity() {
	let path =
		PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/gate/fixtures/anthropic.golden.json");
	let data = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read golden {}: {e}", path.display()));
	let rows: Vec<Row> = serde_json::from_str(&data).expect("parse golden json");

	let mut failures = Vec::new();
	for row in &rows {
		let got = canonical(&decode_anthropic(&row.chunks, &row.api, &row.provider, &row.model));
		if got != row.expected {
			failures.push(format!(
				"  fixture '{}':\n     expected: {}\n     got:      {}",
				row.name, row.expected, got
			));
		}
	}

	assert!(
		failures.is_empty(),
		"anthropic decoder divergence ({} of {} fixtures):\n{}",
		failures.len(),
		rows.len(),
		failures.join("\n\n")
	);
}
