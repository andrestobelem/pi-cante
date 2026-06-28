// Golden generator for the Anthropic byte-level SSE-decoder contract gate.
//
//   npm run gate:gen:anthropic -w @earendil-works/pi-ai        -> writes fixtures/anthropic.golden.json
//   npm run gate:selfcheck:anthropic -w @earendil-works/pi-ai  -> regenerates in memory, diffs vs committed
//
// The TS decoder is the oracle: golden.expected is the canonical transcript. The Rust decode_anthropic
// must reproduce it byte-for-byte from the same recorded byte chunks.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { fixtures } from "./anthropic-corpus.ts";
import { decodeTranscript, toByteChunks } from "./anthropic-replay.ts";
import { canonicalize } from "./canonical.ts";

type GoldenRow = {
	name: string;
	api: string;
	provider: string;
	model: string;
	chunks: number[][];
	expected: string;
};

async function generate(): Promise<GoldenRow[]> {
	const rows: GoldenRow[] = [];
	for (const fixture of fixtures) {
		const chunks = toByteChunks(fixture.chunks);
		const { api, provider, model, transcript } = await decodeTranscript(chunks);
		rows.push({ name: fixture.name, api, provider, model, chunks, expected: canonicalize(transcript) });
	}
	return rows;
}

const goldenPath = fileURLToPath(new URL("./fixtures/anthropic.golden.json", import.meta.url));
const rows = await generate();
const serialized = `${JSON.stringify(rows, null, 2)}\n`;

if (process.argv.includes("--check")) {
	let committed: string;
	try {
		committed = readFileSync(goldenPath, "utf8");
	} catch {
		console.error(`[gate] missing golden ${goldenPath}; run gate:gen:anthropic`);
		process.exit(1);
	}
	if (committed !== serialized) {
		console.error("[gate] anthropic golden drift: TS decoder output no longer matches the committed golden.");
		console.error(
			"[gate] regenerate (npm run gate:gen:anthropic -w @earendil-works/pi-ai), re-review, commit fixture + Rust together.",
		);
		process.exit(1);
	}
	console.log(`[gate] anthropic golden up to date (${rows.length} fixtures)`);
} else {
	writeFileSync(goldenPath, serialized);
	console.log(`[gate] wrote ${rows.length} fixtures -> ${goldenPath}`);
}
