//! The only network client in Tidy. Called only by the explicit install action.
use crate::catalog::ModelSpec;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};
#[derive(Clone)]
pub struct ModelStore {
    root: PathBuf,
}
impl ModelStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    pub fn path(&self, model: &ModelSpec) -> PathBuf {
        self.root.join(format!("{}.gguf", model.id))
    }
    pub fn present(&self, model: &ModelSpec) -> bool {
        fs::symlink_metadata(self.path(model))
            .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink() && m.len() == model.bytes)
    }
    pub fn verify(&self, model: &ModelSpec, cancel: &AtomicBool) -> Result<PathBuf, String> {
        let path = self.path(model);
        verify_file(&path, model.bytes, &model.sha256, cancel)?;
        Ok(path)
    }
    pub fn install(
        &self,
        model: &ModelSpec,
        cancel: &AtomicBool,
        progress: &AtomicU64,
    ) -> Result<PathBuf, String> {
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        if self.present(model) && self.verify(model, cancel).is_ok() {
            progress.store(model.bytes, Ordering::Relaxed);
            return Ok(self.path(model));
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("Installation cancelled".into());
        }
        let client = reqwest::blocking::Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let allowed = attempt.url().host_str().is_some_and(|host| {
                    host == "huggingface.co"
                        || host.ends_with(".huggingface.co")
                        || host.ends_with(".hf.co")
                });
                if attempt.previous().len() > 8 || !allowed || attempt.url().scheme() != "https" {
                    attempt.error("Untrusted model download redirect")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|e| e.to_string())?;
        let response = client
            .get(&model.url)
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("Model download failed: {e}"))?;
        if response.content_length().is_some_and(|n| n != model.bytes) {
            return Err("Download size differs from pinned model metadata".into());
        }
        self.install_reader(model, response, cancel, progress)
    }
    pub fn install_reader(
        &self,
        model: &ModelSpec,
        mut input: impl Read,
        cancel: &AtomicBool,
        progress: &AtomicU64,
    ) -> Result<PathBuf, String> {
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let partial = self.root.join(format!(
            ".{}-{}-{unique}.part",
            model.id,
            std::process::id()
        ));
        let cleanup = Partial(partial.clone());
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&partial).map_err(|e| e.to_string())?;
        let mut digest = Sha256::new();
        let mut size = 0u64;
        let mut buffer = [0u8; 65536];
        let mut header = Vec::new();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("Installation cancelled".into());
            }
            let count = input
                .read(&mut buffer)
                .map_err(|e| format!("Download interrupted: {e}"))?;
            if count == 0 {
                break;
            }
            size += count as u64;
            if size > model.bytes {
                return Err("Model exceeds pinned size".into());
            }
            if header.len() < 4 {
                header.extend_from_slice(&buffer[..count.min(4 - header.len())]);
            }
            file.write_all(&buffer[..count])
                .map_err(|e| e.to_string())?;
            digest.update(&buffer[..count]);
            progress.store(size, Ordering::Relaxed);
        }
        if header != b"GGUF"
            || size != model.bytes
            || format!("{:x}", digest.finalize()) != model.sha256
        {
            return Err(
                "Model verification failed (format, size, or SHA-256); nothing installed".into(),
            );
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("Installation cancelled".into());
        }
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        let destination = self.path(model);
        // Never overwrite even an invalid existing installation. The explicit install action
        // keeps it as a recoverable app-owned backup before atomically publishing the verified file.
        if fs::symlink_metadata(&destination).is_ok() {
            fs::rename(
                &destination,
                self.root.join(format!("{}.backup-{unique}", model.id)),
            )
            .map_err(|e| e.to_string())?;
        }
        fs::rename(&partial, &destination).map_err(|e| e.to_string())?;
        drop(cleanup);
        Ok(destination)
    }
}
struct Partial(PathBuf);
impl Drop for Partial {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
pub fn verify_file(
    path: &Path,
    expected_size: u64,
    expected_hash: &str,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "Model or inference worker is missing".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != expected_size {
        return Err("Installed file has an unexpected type or size".into());
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut read = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        read += n as u64;
        if read > expected_size {
            return Err("Installed file changed during verification".into());
        }
        hash.update(&buffer[..n]);
    }
    if read != expected_size || format!("{:x}", hash.finalize()) != expected_hash {
        return Err(
            "Checksum mismatch; reinstall the model or rebuild the inference worker".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "tidy-model-tests-{}-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn spec() -> ModelSpec {
        ModelSpec {
            id: "fixture".into(),
            name: "Test fixture".into(),
            bytes: 8,
            sha256: String::new(),
            url: "https://example.invalid".into(),
            license: "MIT".into(),
            source: "test".into(),
            custom: false,
        }
    }
    fn model_spec() -> ModelSpec {
        ModelSpec {
            sha256: "707e858183f1bb2cbf58e1ef07c195adac4bf20bfc9a5827f63f17170fc78f8b".into(),
            ..spec()
        }
    }
    // Compute known fixture's digest at runtime to avoid coupling test data to a handwritten hash.
    fn install(store: &ModelStore, data: &[u8], cancel: bool) -> Result<PathBuf, String> {
        let hash = format!("{:x}", Sha256::digest(b"GGUFtest"));
        let mut model = model_spec();
        model.sha256 = hash;
        store.install_reader(&model, data, &AtomicBool::new(cancel), &AtomicU64::new(0))
    }
    #[test]
    fn publishes_only_verified_bytes_and_can_verify_after_restart() {
        let f = Fixture::new();
        let store = ModelStore::new(f.0.clone());
        let path = install(&store, b"GGUFtest", false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"GGUFtest");
        assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
        let reopened = ModelStore::new(f.0.clone());
        let hash = format!("{:x}", Sha256::digest(b"GGUFtest"));
        verify_file(&reopened.path(&spec()), 8, &hash, &AtomicBool::new(false)).unwrap();
    }
    #[test]
    fn corrupt_truncated_and_oversized_models_are_not_installed() {
        for data in [
            b"GGUFbad!".as_slice(),
            b"GGUF".as_slice(),
            b"GGUFtoolong".as_slice(),
        ] {
            let f = Fixture::new();
            let store = ModelStore::new(f.0.clone());
            assert!(install(&store, data, false).is_err());
            assert_eq!(fs::read_dir(&f.0).unwrap().count(), 0);
        }
    }
    #[test]
    fn cancelled_install_leaves_no_final_or_partial_file() {
        let f = Fixture::new();
        assert!(install(&ModelStore::new(f.0.clone()), b"GGUFtest", true).is_err());
        assert_eq!(fs::read_dir(&f.0).unwrap().count(), 0);
    }
    #[test]
    fn failed_replacement_preserves_previous_file() {
        let f = Fixture::new();
        let store = ModelStore::new(f.0.clone());
        let path = install(&store, b"GGUFtest", false).unwrap();
        assert!(install(&store, b"GGUFbad!", false).is_err());
        assert_eq!(fs::read(path).unwrap(), b"GGUFtest");
    }
    #[test]
    fn same_size_corruption_is_detected() {
        let f = Fixture::new();
        let store = ModelStore::new(f.0.clone());
        let path = install(&store, b"GGUFtest", false).unwrap();
        fs::write(&path, b"GGUFbad!").unwrap();
        let hash = format!("{:x}", Sha256::digest(b"GGUFtest"));
        assert!(verify_file(&path, 8, &hash, &AtomicBool::new(false)).is_err());
    }
    #[test]
    fn missing_model_is_unavailable() {
        let f = Fixture::new();
        let store = ModelStore::new(f.0.clone());
        assert!(!store.present(&spec()));
        assert!(store.verify(&spec(), &AtomicBool::new(false)).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn installed_model_symlinks_are_refused() {
        let f = Fixture::new();
        fs::write(f.0.join("real"), b"GGUFtest").unwrap();
        let store = ModelStore::new(f.0.clone());
        std::os::unix::fs::symlink(f.0.join("real"), store.path(&spec())).unwrap();
        assert!(!store.present(&spec()));
        assert!(store.verify(&spec(), &AtomicBool::new(false)).is_err());
    }
}
