use serde::Serialize;
#[derive(Debug, Clone, Serialize)]
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub url: &'static str,
    pub license: &'static str,
    pub source: &'static str,
}
pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "qwen3-1.7b-q4",
        name: "Qwen3 1.7B · Q4_K_M",
        bytes: 1_282_439_264,
        sha256: "d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5",
        url: "https://huggingface.co/ggml-org/Qwen3-1.7B-GGUF/resolve/daeb8e2d528a760970442092f6bf1e55c3b659eb/Qwen3-1.7B-Q4_K_M.gguf",
        license: "Apache-2.0",
        source: "https://huggingface.co/ggml-org/Qwen3-1.7B-GGUF/tree/daeb8e2d528a760970442092f6bf1e55c3b659eb",
    },
    ModelSpec {
        id: "qwen3-0.6b-q4",
        name: "Qwen3 0.6B · Q4_0",
        bytes: 428_970_080,
        sha256: "da2572f16c06133561ce56accaa822216f2391ef4d37fba427801cd6736417d4",
        url: "https://huggingface.co/ggml-org/Qwen3-0.6B-GGUF/resolve/b5f37287796e5be0ea3dab2e7430873fb3f73e49/Qwen3-0.6B-Q4_0.gguf",
        license: "Apache-2.0",
        source: "https://huggingface.co/ggml-org/Qwen3-0.6B-GGUF/tree/b5f37287796e5be0ea3dab2e7430873fb3f73e49",
    },
];
pub fn model(id: &str) -> Result<&'static ModelSpec, String> {
    MODELS
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| "Unknown model; choose a catalog entry".into())
}
