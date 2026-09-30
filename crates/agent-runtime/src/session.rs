//! One bounded, local process per investigation. No filesystem execution authority.
use crate::{
    store::verify_file,
    worker::{Answer, Backend, Worker},
};
use serde_json::json;
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
pub struct Session {
    child: Child,
    input: ChildStdin,
    output: Receiver<Result<Answer, String>>,
    reader: Option<JoinHandle<()>>,
}
impl Session {
    pub fn start(
        worker: &Worker,
        model: PathBuf,
        backend: Backend,
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        verify_file(&worker.executable, worker.bytes, &worker.sha256, cancel)?;
        let mut command = Command::new(&worker.executable);
        command
            .arg("--model")
            .arg(model)
            .arg("--backend")
            .arg(match backend {
                Backend::Cpu => "cpu",
                Backend::Metal => "metal",
            })
            .arg("--session");
        command.env_clear();
        #[cfg(windows)]
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = child.stdin.take().ok_or("Inference stdin missing")?;
        let stdout = child.stdout.take().ok_or("Inference stdout missing")?;
        let (send, output) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut stream = BufReader::new(stdout);
            for _ in 0..25 {
                let mut bytes = Vec::new();
                let read = stream.by_ref().take(32769).read_until(b'\n', &mut bytes);
                match read {
                    Ok(0) => break,
                    Ok(_) if bytes.len() > 32768 => {
                        let _ = send.send(Err("Inference response exceeded limit".into()));
                        break;
                    }
                    Ok(_) => {
                        if send
                            .send(serde_json::from_slice(&bytes).map_err(|e| e.to_string()))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = send.send(Err(e.to_string()));
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            input,
            output,
            reader: Some(reader),
        })
    }
    pub fn request(
        &mut self,
        prompt: &str,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Answer, String> {
        if prompt.is_empty() || prompt.len() > 9000 {
            return Err("Agent context exceeded its bounded window".into());
        }
        let payload = serde_json::to_vec(
            &json!({"version":1,"prompt":prompt,"max_tokens":1536,"agent_only":true}),
        )
        .map_err(|e| e.to_string())?;
        self.input
            .write_all(&payload)
            .and_then(|_| self.input.write_all(b"\n"))
            .and_then(|_| self.input.flush())
            .map_err(|e| e.to_string())?;
        let start = Instant::now();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("Planning cancelled".into());
            }
            if start.elapsed() >= timeout {
                return Err("Planning step timed out".into());
            }
            match self.output.recv_timeout(Duration::from_millis(100)) {
                Ok(answer) => {
                    return answer.and_then(|a| {
                        if a.truncated {
                            Err("Model response was incomplete; shorten the request".into())
                        } else {
                            Ok(a)
                        }
                    });
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(format!(
                        "Local worker ended before answering (status {:?})",
                        self.child.try_wait().ok().flatten()
                    ));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Drain the bounded channel before joining so a final send cannot deadlock.
        while self.output.try_recv().is_ok() {}
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
