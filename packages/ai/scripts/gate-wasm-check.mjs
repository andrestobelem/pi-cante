// gate-wasm-check.mjs — staleness guard for the committed WASM integration-gate artifact
// (packages/ai/test/gate/wasm/). LOCAL/MANUAL ONLY — needs Rust + the wasm32-unknown-unknown
// target + wasm-bindgen, which CI does not have. NOT part of `npm test` or `npm run check`.
//
// DEFAULT (behavioral re-equivalence): rebuild the wasm into a temp dir from the CURRENT Rust
// source and re-run all 51 golden rows through it, asserting each still === expected. This is the
// trustworthy staleness signal: it proves a freshly-built wasm still reproduces the goldens.
//
// `--bytes` (opt-in, same-machine/same-toolchain only): additionally byte-compares each freshly
// built file against the committed copy. wasm-bindgen output is NOT bit-reproducible across
// rustc / wasm-bindgen / binaryen versions or across machines, so a byte diff is meaningful ONLY
// under the pinned rust-toolchain.toml + the pinned wasm-bindgen-cli on the same box. Treat a
// byte diff as "rebuild + recommit", never as a hard cross-machine gate.
//
// What NEITHER mode catches: Rust changes on behavior no golden exercises (see WASM_GATE.md).

import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url)); // packages/ai/scripts
const aiRoot = dirname(here); // packages/ai
const manifest = join(aiRoot, "native/ai-streaming-core/Cargo.toml");
const crateDir = dirname(manifest);
const committedDir = join(aiRoot, "test/gate/wasm");
const fixturesDir = join(aiRoot, "test/gate/fixtures");
const wantBytes = process.argv.includes("--bytes");

function toolMissing(cmd, args) {
	try {
		execFileSync(cmd, args, { stdio: "ignore" });
		return false;
	} catch {
		return true;
	}
}

if (toolMissing("cargo", ["--version"]) || toolMissing("wasm-bindgen", ["--version"])) {
	console.error(
		"gate:wasm:check requires Rust + the wasm32-unknown-unknown target + wasm-bindgen on PATH\n" +
			"(local-only guard; ~/.cargo/bin must be on PATH). See native/ai-streaming-core/WASM_GATE.md.",
	);
	process.exit(1);
}

const tmp = mkdtempSync(join(tmpdir(), "pi-wasm-gate-"));
console.log(`[gate:wasm:check] rebuilding wasm from current src into ${tmp}`);
execFileSync("cargo", ["build", "--manifest-path", manifest, "--target", "wasm32-unknown-unknown", "--release"], {
	stdio: "inherit",
});
const builtWasm = join(crateDir, "target/wasm32-unknown-unknown/release/ai_streaming_core.wasm");
execFileSync("wasm-bindgen", ["--target", "nodejs", "--out-dir", tmp, builtWasm], { stdio: "inherit" });
// Sidecar so the freshly built glue loads as CommonJS under this "type":"module" repo.
writeFileSync(join(tmp, "package.json"), '{\n\t"type": "commonjs"\n}\n');

const requireCjs = createRequire(import.meta.url);
const fresh = requireCjs(join(tmp, "ai_streaming_core.js"));
const anthropic = JSON.parse(readFileSync(join(fixturesDir, "anthropic.golden.json"), "utf8"));
const openai = JSON.parse(readFileSync(join(fixturesDir, "openai.golden.json"), "utf8"));

let drift = 0;
for (const row of anthropic) {
	const got = fresh.decode_anthropic_canonical(JSON.stringify(row.chunks), row.api, row.provider, row.model);
	if (got !== row.expected) {
		drift++;
		console.error(`  BEHAVIORAL DRIFT  anthropic '${row.name}'`);
	}
}
for (const row of openai) {
	const got = fresh.decode_openai_canonical(JSON.stringify(row.chunks), row.api, row.provider, row.model);
	if (got !== row.expected) {
		drift++;
		console.error(`  BEHAVIORAL DRIFT  openai '${row.name}'`);
	}
}
if (drift > 0) {
	console.error(`[gate:wasm:check] ${drift} row(s) drifted: a freshly built wasm disagrees with the goldens.`);
	process.exit(1);
}
console.log(`[gate:wasm:check] behavioral: freshly built wasm reproduces all ${anthropic.length + openai.length} goldens ✓`);

if (wantBytes) {
	let byteDiffs = 0;
	for (const file of readdirSync(tmp)) {
		if (file === "package.json") continue;
		const committed = join(committedDir, file);
		if (!existsSync(committed)) {
			byteDiffs++;
			console.error(`  --bytes: committed artifact missing: ${file}`);
			continue;
		}
		if (Buffer.compare(readFileSync(join(tmp, file)), readFileSync(committed)) !== 0) {
			byteDiffs++;
			console.error(`  --bytes: ${file} differs from committed`);
		}
	}
	if (byteDiffs > 0) {
		console.error(
			"[gate:wasm:check] --bytes: committed artifacts are STALE vs a fresh build under THIS toolchain.\n" +
				"Rebuild + recommit (note: byte-identity holds only same-machine/same-toolchain).",
		);
		process.exit(1);
	}
	console.log("[gate:wasm:check] --bytes: committed artifacts byte-identical to a fresh build under this toolchain ✓");
}
