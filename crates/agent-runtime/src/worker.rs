use crate::store::verify_file;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Cpu,
    Metal,
}
#[derive(Debug, Serialize)]
struct Request<'a> {
    version: u8,
    prompt: &'a str,
    max_tokens: u32,
    #[serde(skip_serializing_if = "is_false")]
    rules_only: bool,
}
fn is_false(value: &bool) -> bool {
    !*value
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub version: u8,
    pub text: String,
    pub tokens: u32,
    pub elapsed_ms: u64,
    pub backend: String,
    pub truncated: bool,
}
#[derive(Clone)]
pub struct Worker {
    pub executable: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}
impl Worker {
    pub fn run(
        &self,
        model: PathBuf,
        prompt: &str,
        backend: Backend,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Answer, String> {
        self.run_format(model, prompt, backend, cancel, timeout, false)
    }
    pub fn run_rules(
        &self,
        model: PathBuf,
        prompt: &str,
        backend: Backend,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Answer, String> {
        self.run_format(model, prompt, backend, cancel, timeout, true)
    }
    fn run_format(
        &self,
        model: PathBuf,
        prompt: &str,
        backend: Backend,
        cancel: &AtomicBool,
        timeout: Duration,
        rules_only: bool,
    ) -> Result<Answer, String> {
        if prompt.is_empty() || prompt.len() > 12000 {
            return Err("Prompt must contain 1–12,000 UTF-8 bytes".into());
        }
        if matches!(backend, Backend::Metal) && !cfg!(target_os = "macos") {
            return Err("Metal is only supported on macOS".into());
        }
        verify_file(&self.executable, self.bytes, &self.sha256, cancel)?;
        let payload = serde_json::to_vec(&Request {
            version: 1,
            prompt,
            max_tokens: if rules_only { 1024 } else { 384 },
            rules_only,
        })
        .map_err(|e| e.to_string())?;
        let mut command = Command::new(&self.executable);
        command
            .arg("--model")
            .arg(model)
            .arg("--backend")
            .arg(match backend {
                Backend::Cpu => "cpu",
                Backend::Metal => "metal",
            });
        // The model never selects executable names, flags, environment, paths or tools.
        command.env_clear();
        #[cfg(windows)]
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let child = command
            .spawn()
            .map_err(|e| format!("Could not start local inference: {e}"))?;
        supervise(child, payload, cancel, timeout.min(Duration::from_secs(90)))
    }
}
fn supervise(
    mut child: std::process::Child,
    payload: Vec<u8>,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Answer, String> {
    let mut stdin = child.stdin.take().ok_or("Worker stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("Worker stdout unavailable")?;
    let overflow = AtomicBool::new(false);
    let start = Instant::now();
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || {
            stdin.write_all(&payload)?;
            stdin.flush()
        });
        let reader = scope.spawn(|| {
            let mut bytes = Vec::new();
            let result = stdout.take(32769).read_to_end(&mut bytes);
            if bytes.len() > 32768 {
                overflow.store(true, Ordering::Relaxed);
            }
            result.map(|_| bytes)
        });
        let status = loop {
            let failure = if cancel.load(Ordering::Relaxed) {
                Some("Inference cancelled")
            } else if start.elapsed() >= timeout {
                Some("Inference timed out; try a smaller model or Metal")
            } else if overflow.load(Ordering::Relaxed) {
                Some("Worker output exceeded limit")
            } else {
                None
            };
            if let Some(message) = failure {
                let _ = child.kill();
                let _ = child.wait();
                break Err(message.to_string());
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(e.to_string());
                }
            }
        };
        let output = reader
            .join()
            .map_err(|_| "Worker output reader failed")?
            .map_err(|e| e.to_string())?;
        let write_result = writer.join().map_err(|_| "Worker input writer failed")?;
        let status = status?;
        if overflow.load(Ordering::Relaxed) {
            return Err("Worker output exceeded limit".into());
        }
        if !status.success() {
            return Err(format!(
                "Local inference worker stopped ({status}); the model may be unsupported or memory may be insufficient. Scanning remains available."
            ));
        }
        write_result.map_err(|e| e.to_string())?;
        parse_answer(&output)
    })
}
pub fn parse_answer(bytes: &[u8]) -> Result<Answer, String> {
    if bytes.len() > 32768 {
        return Err("Worker output exceeded limit".into());
    }
    let answer: Answer =
        serde_json::from_slice(bytes).map_err(|_| "Malformed inference response".to_string())?;
    if answer.version != 1
        || answer.tokens > 384
        || answer.text.len() > 16384
        || answer.text.trim().is_empty()
        || !matches!(answer.backend.as_str(), "cpu" | "metal")
    {
        return Err("Inference response violates limits".into());
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn child(mode: &str) -> std::process::Child {
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "worker::tests::process_fixture",
                "--nocapture",
            ])
            .env("TIDY_TEST_WORKER_MODE", mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }
    #[test]
    #[ignore = "child-process fixture; launched only by supervisor tests"]
    fn process_fixture() {
        match std::env::var("TIDY_TEST_WORKER_MODE").as_deref() {
            Ok("sleep") => std::thread::sleep(Duration::from_secs(5)),
            Ok("crash") => std::process::exit(42),
            Ok("flood") => {
                let _ = std::io::stdout().write_all(&vec![b'x'; 100_000]);
            }
            _ => panic!("fixture must be launched by a supervisor test"),
        }
    }
    #[test]
    fn timeout_kills_and_reaps_a_stalled_worker_even_if_stdin_is_blocked() {
        let start = Instant::now();
        let error = supervise(
            child("sleep"),
            vec![b'x'; 1_000_000],
            &AtomicBool::new(false),
            Duration::from_millis(80),
        )
        .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn cancellation_kills_worker() {
        let start = Instant::now();
        let error = supervise(
            child("sleep"),
            vec![],
            &AtomicBool::new(true),
            Duration::from_secs(3),
        )
        .unwrap_err();
        assert!(error.contains("cancelled"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn crash_is_an_error_not_a_response() {
        assert!(
            supervise(
                child("crash"),
                vec![],
                &AtomicBool::new(false),
                Duration::from_secs(3)
            )
            .unwrap_err()
            .contains("stopped")
        );
    }
    #[test]
    fn stdout_is_bounded() {
        assert!(
            supervise(
                child("flood"),
                vec![],
                &AtomicBool::new(false),
                Duration::from_secs(3)
            )
            .unwrap_err()
            .contains("exceeded")
        );
    }
    #[test]
    fn accepts_a_valid_envelope() {
        let answer=parse_answer(br#"{"version":1,"text":"A local answer.","tokens":4,"elapsed_ms":100,"backend":"cpu","truncated":false}"#).unwrap();
        assert_eq!(answer.tokens, 4);
    }
    #[test]
    fn rejects_malformed_unknown_and_over_budget_outputs() {
        for invalid in [
            "not json",
            "{}",
            r#"{"version":1,"text":"x","tokens":4,"elapsed_ms":1,"backend":"cpu","truncated":false,"execute":"rm"}"#,
            r#"{"version":1,"text":"x","tokens":385,"elapsed_ms":1,"backend":"cpu","truncated":false}"#,
            r#"{"version":1,"text":"","tokens":0,"elapsed_ms":1,"backend":"cpu","truncated":false}"#,
            r#"{"version":2,"text":"x","tokens":1,"elapsed_ms":1,"backend":"cpu","truncated":false}"#,
        ] {
            assert!(parse_answer(invalid.as_bytes()).is_err());
        }
        assert!(parse_answer(&vec![b'x'; 32769]).is_err());
    }
    #[test]
    fn invalid_prompts_fail_before_worker_spawn() {
        let worker = Worker {
            executable: PathBuf::from("missing-worker"),
            bytes: 0,
            sha256: String::new(),
        };
        assert!(
            worker
                .run(
                    PathBuf::from("missing-model"),
                    &"x".repeat(12001),
                    Backend::Cpu,
                    &AtomicBool::new(false),
                    Duration::from_secs(1)
                )
                .unwrap_err()
                .contains("Prompt")
        );
    }
}
