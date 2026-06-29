// Increment 4 — WASM FFI ONE-SHOT INTEGRATION GATE.
//
// Proves the SAME Rust decoders (decode_anthropic / decode_openai) + canonical(), when called
// through a real JS<->Rust wasm-bindgen boundary, reproduce the committed golden transcripts
// byte-for-byte. Loads the COMMITTED wasm under ./wasm/ — no Rust toolchain needed, so this runs
// in CI via `npm test`.
//
// WHAT THIS GATE CATCHES:
//   - a committed wasm whose decode logic disagrees with ANY golden row   -> RED
//   - coverage drift (rows asserted != 51, or a corpus is truncated)      -> RED
// WHAT IT DOES *NOT* CATCH (honesty note; see native/ai-streaming-core/WASM_GATE.md):
//   - "stale-but-correct": a wasm built from OLDER Rust that still matches every golden row passes
//     silently. Provenance (committed wasm == build(current src/)) is NOT proven here.
//   - Rust changes on behavior no golden exercises.
//   Only `npm run gate:wasm:check` (manual, needs the Rust + wasm32 toolchain) closes that gap.
//
// NOTE on input fidelity: the boundary input path (JS JSON.parse(golden) -> JSON.stringify(row.chunks)
// -> serde_json::from_str) is NOT byte-identical to the native conformance test's path (it reads the
// golden text straight into serde). It is verified LOSSLESS for the current corpus; a future fixture
// with a float / exponent / >2^53 integer / -0 would need a fidelity re-check. See WASM_GATE.md.

import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

type GoldenRow = {
	name: string;
	api: string;
	provider: string;
	model: string;
	chunks: unknown[];
	expected: string;
};

type WasmGate = {
	decode_anthropic_canonical(chunksJson: string, api: string, provider: string, model: string): string;
	decode_openai_canonical(chunksJson: string, api: string, provider: string, model: string): string;
};

const here = dirname(fileURLToPath(import.meta.url));
const requireCjs = createRequire(import.meta.url);
const wasm = requireCjs(join(here, "wasm", "ai_streaming_core.js")) as WasmGate;

const anthropicGolden = JSON.parse(
	readFileSync(join(here, "fixtures", "anthropic.golden.json"), "utf8"),
) as GoldenRow[];
const openaiGolden = JSON.parse(readFileSync(join(here, "fixtures", "openai.golden.json"), "utf8")) as GoldenRow[];

let rowsAsserted = 0;

describe("wasm FFI integration gate (decoders crossing the real JS<->Rust boundary)", () => {
	it("exercises the full committed corpus (no silent truncation)", () => {
		expect(anthropicGolden.length).toBe(13);
		expect(openaiGolden.length).toBe(38);
	});

	for (const row of anthropicGolden) {
		it(`anthropic '${row.name}' reproduces golden across the boundary`, () => {
			// u8 sanity: surface a future non-u8 fixture as a readable JS assertion instead of an
			// opaque serde JsError thrown from across the wasm boundary.
			for (const chunk of row.chunks as number[][]) {
				for (const b of chunk) {
					expect(Number.isInteger(b) && b >= 0 && b <= 255).toBe(true);
				}
			}
			const out = wasm.decode_anthropic_canonical(JSON.stringify(row.chunks), row.api, row.provider, row.model);
			expect(out).toBe(row.expected);
			rowsAsserted++;
		});
	}

	for (const row of openaiGolden) {
		it(`openai '${row.name}' reproduces golden across the boundary`, () => {
			const out = wasm.decode_openai_canonical(JSON.stringify(row.chunks), row.api, row.provider, row.model);
			expect(out).toBe(row.expected);
			rowsAsserted++;
		});
	}

	it("asserted exactly 51 rows (13 anthropic + 38 openai)", () => {
		expect(rowsAsserted).toBe(51);
	});
});
