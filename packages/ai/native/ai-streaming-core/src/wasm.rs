//! Increment 4: thin wasm-bindgen FFI shim over the UNCHANGED pure decoders.
//!
//! Exists only so the JS integration gate (packages/ai/test/gate/wasm-integration.test.ts)
//! can exercise `decode_anthropic` / `decode_openai` / `canonical` across a real
//! JS <-> Rust wasm-bindgen boundary. NOT wired into production `src/`. One-shot only:
//! the whole input is marshalled in as a JSON string, the whole canonical transcript comes
//! back as a string. Incremental/push streaming is a later increment.
//!
//! Compiled ONLY for wasm32 (see the cfg-gated `pub mod wasm;` in lib.rs), so the native
//! conformance build (`cargo test --features gate`) never sees wasm-bindgen.

use wasm_bindgen::prelude::*;

use crate::{canonical, decode_anthropic, decode_openai};

/// `chunks_json` = `JSON.stringify` of a golden row's `chunks` (an array of byte-arrays).
/// Deserializes to the exact `Vec<Vec<u8>>` the native conformance test feeds `decode_anthropic`.
#[wasm_bindgen]
pub fn decode_anthropic_canonical(
    chunks_json: &str,
    api: &str,
    provider: &str,
    model: &str,
) -> Result<String, JsError> {
    let chunks: Vec<Vec<u8>> =
        serde_json::from_str(chunks_json).map_err(|e| JsError::new(&e.to_string()))?;
    Ok(canonical(&decode_anthropic(&chunks, api, provider, model)))
}

/// `chunks_json` = `JSON.stringify` of a golden row's `chunks` (an array of chunk objects).
/// Deserializes to the exact `Vec<serde_json::Value>` the native conformance test feeds `decode_openai`.
#[wasm_bindgen]
pub fn decode_openai_canonical(
    chunks_json: &str,
    api: &str,
    provider: &str,
    model: &str,
) -> Result<String, JsError> {
    let chunks: Vec<serde_json::Value> =
        serde_json::from_str(chunks_json).map_err(|e| JsError::new(&e.to_string()))?;
    Ok(canonical(&decode_openai(&chunks, api, provider, model)))
}
