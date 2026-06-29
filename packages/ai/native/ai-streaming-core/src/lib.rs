//! Rust streaming-core for the pi `ai` package migration pilot.
//!
//! First increment: a faithful port of `parseStreamingJson` (packages/ai/src/utils/json-parse.ts)
//! plus the `partial-json` 0.1.7 parser it relies on. This is the #1 divergence risk in the
//! streaming-core pilot (tool-call argument reassembly), gated by differential conformance against
//! golden output captured from the TypeScript oracle.

pub mod anthropic;
pub mod openai;
pub mod partial_json;
pub mod sse;

// FFI integration gate (Increment 4): compiled ONLY for wasm32 so the native conformance
// build never pulls in wasm-bindgen. See packages/ai/test/gate/wasm-integration.test.ts.
#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub use anthropic::decode_anthropic;
pub use openai::decode_openai;
pub use partial_json::{canonical, parse_streaming_json, JsVal};
