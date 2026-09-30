//! Builds mock folders for trying Tidy safely (and for the tests):
//!   cargo run --example make_mock -p tidy-agent-runtime -- ~/TidyMock            # small tree
//!   cargo run --example make_mock -p tidy-agent-runtime -- ~/TidyMock --large    # ~1000 messy files
#[path = "../../../tests/support/large_mock.rs"]
mod large_mock;
#[path = "../../../tests/support/mock.rs"]
mod mock;
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(target) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("usage: make_mock <new folder> [--large]");
        std::process::exit(2);
    };
    let path = std::path::PathBuf::from(target);
    if path.exists()
        && std::fs::read_dir(&path)
            .map(|mut d| d.next().is_some())
            .unwrap_or(true)
    {
        eprintln!("{target} already exists and is not empty; choose a new folder");
        std::process::exit(1);
    }
    std::fs::create_dir_all(&path).unwrap();
    if args.iter().any(|a| a == "--large") {
        let m = large_mock::build_large(&path, 1000, 20260930);
        println!(
            "Large mock ready: {target} ({} files; Inbox/ is the messy dump)",
            m.total
        );
    } else {
        mock::build(&path);
        println!("Mock folder ready: {target}");
    }
}
