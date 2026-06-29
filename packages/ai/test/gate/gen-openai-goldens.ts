// Golden generator for the OpenAI / Ollama chunk-object SSE-decoder contract gate.
//
//   npm run gate:gen:openai -w @earendil-works/pi-ai        -> writes fixtures/openai.golden.json
//   npm run gate:selfcheck:openai -w @earendil-works/pi-ai  -> regenerates in memory, diffs vs committed
//
// The TS decoder is the oracle: golden.expected is the canonical transcript. The Rust decode_openai must
// reproduce it byte-for-byte from the same recorded chunk objects (deserialized as serde_json::Value).

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { canonicalize } from "./canonical.ts";
import { fixtures } from "./openai-corpus.ts";
import { decodeTranscript } from "./openai-replay.ts";

type GoldenRow = {
	name: string;
	api: string;
	provider: string;
	model: string;
	chunks: unknown[];
	expected: string;
};

async function generate(): Promise<GoldenRow[]> {
	const rows: GoldenRow[] = [];
	for (const fixture of fixtures) {
		const { api, provider, model, transcript } = await decodeTranscript(fixture);
		rows.push({
			name: fixture.name,
			api,
			provider,
			model,
			chunks: fixture.chunks,
			expected: canonicalize(transcript),
		});
	}
	return rows;
}

const goldenPath = fileURLToPath(new URL("./fixtures/openai.golden.json", import.meta.url));
const rows = await generate();
const serialized = `${JSON.stringify(rows, null, 2)}\n`;

if (process.argv.includes("--check")) {
	let committed: string;
	try {
		committed = readFileSync(goldenPath, "utf8");
	} catch {
		console.error(`[gate] missing golden ${goldenPath}; run gate:gen:openai`);
		process.exit(1);
	}
	if (committed !== serialized) {
		console.error("[gate] openai golden drift: TS decoder output no longer matches the committed golden.");
		console.error(
			"[gate] regenerate (npm run gate:gen:openai -w @earendil-works/pi-ai), re-review, commit fixture + Rust together.",
		);
		process.exit(1);
	}
	console.log(`[gate] openai golden up to date (${rows.length} fixtures)`);
} else {
	writeFileSync(goldenPath, serialized);
	console.log(`[gate] wrote ${rows.length} fixtures -> ${goldenPath}`);
}
