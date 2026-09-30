use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSpec {
    pub id: String,
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
    pub url: String,
    pub license: String,
    pub source: String,
    /// True for models the user added from Hugging Face.
    #[serde(default)]
    pub custom: bool,
}
fn spec(
    id: &str,
    name: &str,
    bytes: u64,
    sha256: &str,
    url: &str,
    license: &str,
    source: &str,
) -> ModelSpec {
    ModelSpec {
        id: id.into(),
        name: name.into(),
        bytes,
        sha256: sha256.into(),
        url: url.into(),
        license: license.into(),
        source: source.into(),
        custom: false,
    }
}
/// Models bundled in the catalog, each pinned by revision, length and SHA-256.
pub fn builtin() -> Vec<ModelSpec> {
    vec![
        spec(
            "qwen3-4b-instruct-2507-q4",
            "Qwen3 4B Instruct 2507 · Q4_K_M (recommended)",
            2_497_281_120,
            "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597",
            "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/a06e946bb6b655725eafa393f4a9745d460374c9/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
            "Apache-2.0",
            "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/tree/a06e946bb6b655725eafa393f4a9745d460374c9",
        ),
        spec(
            "qwen3-1.7b-q4",
            "Qwen3 1.7B · Q4_K_M",
            1_282_439_264,
            "d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5",
            "https://huggingface.co/ggml-org/Qwen3-1.7B-GGUF/resolve/daeb8e2d528a760970442092f6bf1e55c3b659eb/Qwen3-1.7B-Q4_K_M.gguf",
            "Apache-2.0",
            "https://huggingface.co/ggml-org/Qwen3-1.7B-GGUF/tree/daeb8e2d528a760970442092f6bf1e55c3b659eb",
        ),
        spec(
            "qwen3-0.6b-q4",
            "Qwen3 0.6B · Q4_0",
            428_970_080,
            "da2572f16c06133561ce56accaa822216f2391ef4d37fba427801cd6736417d4",
            "https://huggingface.co/ggml-org/Qwen3-0.6B-GGUF/resolve/b5f37287796e5be0ea3dab2e7430873fb3f73e49/Qwen3-0.6B-Q4_0.gguf",
            "Apache-2.0",
            "https://huggingface.co/ggml-org/Qwen3-0.6B-GGUF/tree/b5f37287796e5be0ea3dab2e7430873fb3f73e49",
        ),
    ]
}
pub fn model(id: &str) -> Result<ModelSpec, String> {
    builtin()
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| "Unknown model; choose a catalog entry".into())
}

const MAX_CUSTOM_BYTES: u64 = 32 * 1024 * 1024 * 1024;
/// Splits a Hugging Face file link into (org/repo, revision, path in repo).
pub fn parse_hugging_face_link(link: &str) -> Result<(String, String, String), String> {
    let link = link.trim();
    let rest = link
        .strip_prefix("https://huggingface.co/")
        .ok_or("Paste a link to a .gguf file on huggingface.co (https://huggingface.co/…)")?;
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.len() < 5 || !matches!(parts[2], "blob" | "resolve") {
        return Err("Open the .gguf file on Hugging Face, then copy its address (it contains /blob/ or /resolve/)".into());
    }
    let path = parts[4..].join("/");
    if !path.to_lowercase().ends_with(".gguf") || path.contains("..") {
        return Err("The link must point to a single .gguf file".into());
    }
    let lower = path.to_lowercase();
    if let Some(at) = lower.find("-of-")
        && lower[..at]
            .rsplit('-')
            .next()
            .is_some_and(|n| n.len() == 5 && n.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(
            "Split (multi-part) GGUF files are not supported. Choose a single-file quantization."
                .into(),
        );
    }
    Ok((
        format!("{}/{}", parts[0], parts[1]),
        parts[3].to_string(),
        path,
    ))
}
/// Looks up a Hugging Face GGUF file and pins it by commit, size and SHA-256 so the normal verified
/// download can install it. Only this metadata request touches the network.
pub fn fetch_hugging_face(link: &str) -> Result<ModelSpec, String> {
    let (repo, revision, path) = parse_hugging_face_link(link)?;
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let get = |url: String| -> Result<serde_json::Value, String> {
        client
            .get(url)
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("Hugging Face lookup failed: {e}"))?
            .text()
            .map_err(|e| format!("Hugging Face lookup failed: {e}"))
            .and_then(|body| {
                serde_json::from_str::<serde_json::Value>(&body)
                    .map_err(|e| format!("Unexpected Hugging Face response: {e}"))
            })
    };
    let info = get(format!(
        "https://huggingface.co/api/models/{repo}/revision/{revision}"
    ))?;
    if info["gated"].as_bool().unwrap_or(false) || info["gated"].is_string() {
        return Err("This model is gated (needs a Hugging Face login), which Tidy does not use. Pick an ungated repository.".into());
    }
    let commit = info["sha"]
        .as_str()
        .ok_or("Hugging Face did not return a commit id")?
        .to_string();
    let folder = path.rsplit_once('/').map(|(d, _)| d.to_string());
    let tree_url = match &folder {
        Some(dir) => format!("https://huggingface.co/api/models/{repo}/tree/{commit}/{dir}"),
        None => format!("https://huggingface.co/api/models/{repo}/tree/{commit}"),
    };
    let tree = get(tree_url)?;
    let entry = tree
        .as_array()
        .and_then(|list| {
            list.iter()
                .find(|e| e["path"].as_str() == Some(path.as_str()))
        })
        .ok_or("That file was not found in the repository")?;
    let sha256 = entry["lfs"]["oid"]
        .as_str()
        .ok_or("Hugging Face lists no checksum for this file")?;
    let bytes = entry["lfs"]["size"]
        .as_u64()
        .or(entry["size"].as_u64())
        .ok_or("Unknown file size")?;
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Unexpected checksum format".into());
    }
    if bytes == 0 || bytes > MAX_CUSTOM_BYTES {
        return Err("Models must be between 1 byte and 32 GB".into());
    }
    let license = info["cardData"]["license"]
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            info["tags"].as_array().and_then(|tags| {
                tags.iter()
                    .filter_map(|t| t.as_str())
                    .find_map(|t| t.strip_prefix("license:").map(str::to_string))
            })
        })
        .unwrap_or_else(|| "see model page".into());
    let file = path.rsplit('/').next().unwrap_or(&path);
    let stem = file.trim_end_matches(".gguf").trim_end_matches(".GGUF");
    let slug: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(40)
        .collect();
    let encoded = path.replace(' ', "%20");
    Ok(ModelSpec {
        id: format!("custom-{slug}-{}", &sha256[..8]),
        name: format!("{stem} · custom"),
        bytes,
        sha256: sha256.to_lowercase(),
        url: format!("https://huggingface.co/{repo}/resolve/{commit}/{encoded}"),
        license,
        source: format!("https://huggingface.co/{repo}/tree/{commit}"),
        custom: true,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hugging_face_links_are_parsed_strictly() {
        let ok = parse_hugging_face_link("https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/blob/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf?download=true").unwrap();
        assert_eq!(ok.0, "unsloth/Qwen3-4B-Instruct-2507-GGUF");
        assert_eq!(ok.1, "main");
        assert_eq!(ok.2, "Qwen3-4B-Instruct-2507-Q4_K_M.gguf");
        for bad in [
            "http://huggingface.co/a/b/blob/main/x.gguf",
            "https://evil.example/a/b/blob/main/x.gguf",
            "https://huggingface.co/a/b/tree/main",
            "https://huggingface.co/a/b/blob/main/model.bin",
            "https://huggingface.co/a/b/blob/main/../x.gguf",
            "https://huggingface.co/a/b/blob/main/m-00001-of-00003.gguf",
        ] {
            assert!(parse_hugging_face_link(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn builtin_catalog_has_unique_ids_and_pinned_hashes() {
        let all = builtin();
        let ids: std::collections::HashSet<_> = all.iter().map(|m| &m.id).collect();
        assert_eq!(ids.len(), all.len());
        assert!(
            all.iter()
                .all(|m| m.sha256.len() == 64 && m.url.starts_with("https://huggingface.co/"))
        );
    }
}
