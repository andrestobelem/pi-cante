# WASM FFI integration gate (Increment 4)

This crate's pure decoders (`decode_anthropic`, `decode_openai`, `canonical`) are validated
**in-process** by the native conformance tests (`cargo test --features gate`, the 13 anthropic +
38 openai + partial-json golden rows). Increment 4 re-asserts the **same equality** with the
**same decoders** and the **same goldens**, but routed through a real **JS ↔ Rust wasm-bindgen
boundary**. If the wasm transcript still matches the golden, the boundary marshalling is faithful.

This is the FFI step of the streaming-core migration: prove the decoders cross the language
boundary correctly, before any later increment wires Rust into production `src/` behind a flag.

## Boundary contract

`src/wasm.rs` (compiled **only** for `wasm32`, behind `#[cfg(target_arch = "wasm32")]`) exposes two
thin shims over the unchanged pure decoders, with a uniform string-in / string-out contract:

```
decode_anthropic_canonical(chunks_json, api, provider, model) -> canonical transcript (String)
decode_openai_canonical(chunks_json, api, provider, model)    -> canonical transcript (String)
```

`chunks_json` is `JSON.stringify` of a golden row's `chunks` field — an array of byte-arrays for
anthropic (`Vec<Vec<u8>>`), an array of chunk objects for openai (`Vec<serde_json::Value>`). The
shim deserializes into the **exact** types the native conformance tests feed the decoders, then
returns `canonical(&decode_*(...))`. Malformed JSON maps to a `JsError` (a catchable JS throw),
never a wasm trap.

The JS gate is `packages/ai/test/gate/wasm-integration.test.ts`. It loads the **committed** wasm
under `test/gate/wasm/` and asserts every golden row reproduces across the boundary. Because the
artifact is committed, the gate runs in CI via `npm test` with **no Rust toolchain**.

## Why the artifact is committed (and the `package.json` sidecar)

`test/gate/wasm/` holds generated, committed files (mirrors `packages/tui`'s committed prebuilt
`.node`, but a single platform-independent `.wasm` — no per-platform matrix):

- `ai_streaming_core.js` — wasm-bindgen `--target nodejs` glue (CommonJS).
- `ai_streaming_core_bg.wasm` — the binary module.
- `ai_streaming_core.d.ts`, `ai_streaming_core_bg.wasm.d.ts` — type stubs.
- `package.json` → `{ "type": "commonjs" }` — **hand-committed sidecar**. `packages/ai` is
  `"type":"module"`, so without this the CJS glue throws `module is not defined in ES module scope`
  when loaded. The sidecar scopes this dir back to CommonJS. It is not matched by the `packages/*`
  workspace glob and has no dependency sections, so it is invisible to workspace tooling.

## Regenerating the artifact

One-time toolchain setup (this is a developer/local step; CI never does it):

```
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.100   # MUST equal the Cargo.toml wasm-bindgen pin
```

Then, from `packages/ai/`:

```
npm run gate:wasm:build
```

which runs `cargo build --target wasm32-unknown-unknown --release` + `wasm-bindgen --target nodejs
--out-dir test/gate/wasm …`. `~/.cargo/bin` must be on `PATH`. The `wasm-bindgen` crate dependency
version (`Cargo.toml`) and the installed `wasm-bindgen-cli` version **must be identical**, or the
`.wasm` will not instantiate. `rust-toolchain.toml` pins the channel + target used.

After a rebuild, the sidecar `package.json` already lives in the out-dir (committed); the raw
two-step CLI does not emit one, so it is preserved.

## Staleness — what is and is NOT caught (no silent caps)

The committed wasm is generated once on a dev box; CI cannot rebuild it. So:

**Caught by the CI vitest gate:** a committed wasm whose decode logic disagrees with any golden
row, and coverage drift (rows asserted ≠ 51 / corpus truncation).

**NOT caught:**

1. **Stale-but-correct.** A wasm built from *older* Rust that still reproduces every golden row
   passes silently, forever. The gate proves *"committed wasm reproduces goldens"*, NOT
   *"committed wasm == build(current `src/`)"*.
2. **Uncovered behavior.** Rust changes on paths no golden exercises are invisible to both the
   vitest gate and the behavioral staleness check below.

**Compensating control (discipline, not enforced):** rebuild + recommit the wasm in the same PR
that touches `native/ai-streaming-core/src/` — mirroring the existing "commit fixture + Rust
together" rule for the golden generators.

**`npm run gate:wasm:check`** (local-only; needs the toolchain) rebuilds into a temp dir and re-runs
all 51 goldens through the freshly built wasm (behavioral re-equivalence — the trustworthy signal).
`-- --bytes` additionally byte-compares against the committed files, but wasm-bindgen output is not
bit-reproducible across toolchain versions/machines, so `--bytes` is meaningful only same-box /
same-toolchain and must never be a hard cross-machine gate.

## Input-path fidelity caveat

The boundary input path is `JS JSON.parse(golden) -> JSON.stringify(row.chunks) -> serde_json::
from_str`, whereas the native test reads the golden text straight into serde. The intermediate JS
re-serialization is a transform the native path never performs. It is verified **lossless for the
current corpus** (all anthropic bytes are integers in 0..=255; all openai chunk numbers round-trip
stably). A future fixture containing a float, an exponent, a `> 2^53` integer, or `-0` could
`JSON.stringify` to a different lexical form — re-check fidelity before relying on the gate for such
a fixture. The test includes a u8 range pre-check to surface a non-u8 anthropic fixture as a
readable assertion rather than an opaque serde error.

## Out of scope (this increment)

Incremental/push/streaming wasm exports; wiring Rust into production `src/` (the `PI_RUST_STREAMING`
swap); NAPI / native `.node`; multi-platform prebuilds; a partial-json wasm export; any CI change
(no Rust toolchain in `ci.yml`). The behavioral gate rides the existing `npm test`; the staleness
guard and `gate:rust` stay manual/local.

## tsgo / biome notes

The generated `.d.ts` matches the root `tsconfig` `packages/*/test/**` include but is safe only
because `skipLibCheck: true` (don't flip that without re-checking the bindgen types). `biome.json`
excludes `packages/ai/test/gate/wasm/**`; the `.js`/`.wasm` are out of biome's `*.ts` scope anyway.
