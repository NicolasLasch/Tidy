use std::{env, process::ExitCode, sync::atomic::AtomicBool};
use tidy_file_indexer::{AuthorizedRoot, ScanLimits, scan};
fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 2 || args[0] != "--root" {
        eprintln!(
            "Usage: tidy-scan --root /absolute/selected/folder\nExplicitly authorizes a read-only metadata scan; no files are changed."
        );
        return ExitCode::from(2);
    }
    let result = AuthorizedRoot::authorize(&args[1])
        .and_then(|root| scan(&root, &ScanLimits::default(), &AtomicBool::new(false)));
    match result {
        Ok(report) => {
            println!(
                "{} files, {} logical bytes, {} omissions, status {:?}",
                report.files.len(),
                report.logical_bytes(),
                report.issues.len(),
                report.stop_reason
            );
            for file in &report.files {
                println!("{:>12} {:?}", file.logical_bytes, file.relative_path);
            }
            for issue in &report.issues {
                eprintln!("Omitted {:?}: {}", issue.relative_path, issue.message);
            }
            if report.stop_reason != tidy_file_indexer::StopReason::Complete
                || !report.issues.is_empty()
            {
                ExitCode::from(3)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("Scan refused for {:?}: {e}", args[1]);
            #[cfg(target_os = "macos")]
            if tidy_platform::os_access_denied(&e) {
                eprintln!(
                    "macOS denied filesystem access. For Downloads, check System Settings > Privacy & Security > Files & Folders > your terminal app > Downloads Folder. Enable access, restart the terminal app, and retry. This may also be a filesystem permission or sandbox restriction; the OS error alone cannot distinguish them."
                );
            }
            ExitCode::FAILURE
        }
    }
}
