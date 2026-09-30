//! Developer harness: explicit installation and synthetic offline smoke tests.
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64},
    time::Duration,
};
use tidy_agent_runtime::{
    catalog::model,
    store::ModelStore,
    worker::{Backend, Worker},
};
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        return Err("Usage: tidy-ai install MODEL_ID DATA_DIR | tidy-ai ask MODEL_ID DATA_DIR WORKER_DIR cpu|metal PROMPT".into());
    }
    let spec = model(&args[1])?;
    let store = ModelStore::new(PathBuf::from(&args[2]));
    let cancel = AtomicBool::new(false);
    match args[0].as_str() {
        "install" => {
            eprintln!(
                "Installing {} ({} bytes), SHA-256 verified",
                spec.name, spec.bytes
            );
            let path = store.install(spec, &cancel, &AtomicU64::new(0))?;
            println!("Installed: {}", path.display());
        }
        "ask" => {
            if args.len() != 6 {
                return Err("ask expects model, data directory, worker directory, backend and one quoted prompt".into());
            }
            let directory = PathBuf::from(&args[3]);
            let manifest: serde_json::Value = serde_json::from_slice(
                &std::fs::read(directory.join("worker-manifest.json"))
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            let worker = Worker {
                executable: directory.join(if cfg!(windows) {
                    "tidy-inference-worker.exe"
                } else {
                    "tidy-inference-worker"
                }),
                bytes: manifest["bytes"]
                    .as_u64()
                    .ok_or("Invalid worker manifest")?,
                sha256: manifest["sha256"]
                    .as_str()
                    .ok_or("Invalid worker manifest")?
                    .into(),
            };
            let backend = match args[4].as_str() {
                "cpu" => Backend::Cpu,
                "metal" => Backend::Metal,
                _ => return Err("Use cpu or metal".into()),
            };
            let answer = worker.run(
                store.verify(spec, &cancel)?,
                &args[5],
                backend,
                &cancel,
                Duration::from_secs(90),
            )?;
            println!(
                "{}",
                serde_json::to_string(&answer).map_err(|e| e.to_string())?
            );
        }
        _ => return Err("Unknown command".into()),
    }
    Ok(())
}
