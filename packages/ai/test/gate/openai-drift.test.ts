// TS self-conformance (drift gate) for the OpenAI / Ollama chunk-object decoder: regenerate the transcript
// from the CURRENT TS decoder and assert it still matches the committed golden. Failure means the TS decoder
// changed and the golden + Rust port must be re-reviewed together in the same PR.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { canonicalize } from "./canonical.ts";
import { fixtures } from "./openai-corpus.ts";
import { decodeTranscript } from "./openai-replay.ts";

type GoldenRow = { name: string; api: string; provider: string; model: string; chunks: unknown[]; expected: string };

const goldenPath = fileURLToPath(new URL("./fixtures/openai.golden.json", import.meta.url));
const golden = JSON.parse(readFileSync(goldenPath, "utf8")) as GoldenRow[];
const byName = new Map(golden.map((row) => [row.name, row]));

describe("openai decoder contract gate (TS self-conformance)", () => {
	it("covers every committed golden fixture", () => {
		expect(fixtures.map((f) => f.name).sort()).toEqual(golden.map((r) => r.name).sort());
	});

	for (const fixture of fixtures) {
		it(`'${fixture.name}' matches committed golden`, async () => {
			const row = byName.get(fixture.name);
			expect(row, `golden missing fixture '${fixture.name}' — regenerate gen-openai-goldens`).toBeDefined();
			const { transcript } = await decodeTranscript(fixture);
			expect(canonicalize(transcript)).toBe(row?.expected);
		});
	}
});
