# Tidy 🧹

[![CI - Build & Test](https://github.com/NicolasLasch/Tidy/actions/workflows/ci.yml/badge.svg)](https://github.com/NicolasLasch/Tidy/actions/workflows/ci.yml)
[![CD - Release Packages](https://github.com/NicolasLasch/Tidy/actions/workflows/cd.yml/badge.svg)](https://github.com/NicolasLasch/Tidy/actions/workflows/cd.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust: 1.90+](https://img.shields.io/badge/Rust-1.90%2B-orange.svg)](https://www.rust-lang.org/)
[![Tauri: 2.x](https://img.shields.io/badge/Tauri-2.x-blue.svg)](https://tauri.app/)
[![React: 19](https://img.shields.io/badge/React-19-61dafb.svg)](https://react.dev/)

**Tidy** is a private, local-first desktop assistant designed to inspect, understand, organize, and clean up files with complete safety and cryptographic guarantees.

Built natively in Rust and Tauri 2, Tidy runs entirely on your local machine. It requires **no accounts**, **no paid cloud APIs**, **no telemetry**, and **no mandatory Python runtime**. All neural intelligence runs offline via an embedded, quantized inference engine powered by Apple Metal GPU and multi-threaded CPU acceleration.

---

## Key Principles & Safety Guarantees

- 🔒 **100% Private & Offline**: All metadata scanning, content analysis, and AI reasoning occur locally on your device. Once installed, Tidy makes zero outbound network calls.
- 🛡️ **Three-Stage Safety Gate**: Every planned filesystem change traverses **Pre-Validation** ➔ **Interactive Preview** ➔ **One-Time Cryptographic Approval**.
- 📝 **Durable SQLite Transaction Journal**: Every planned operation is committed to an atomic Write-Ahead Logging (WAL) transaction journal before execution, with automatic crash recovery.
- 🗑️ **Zero Permanent Deletion**: Removals route through the native operating system Trash (`trash-rs` / AppleScript) with preserved recovery metadata. Files can always be retrieved.
- ↩️ **Verified Reversible Undo**: Any executed transaction can be reviewed in the History tab and restored with cryptographic verification.
- 🚫 **No Overwrite / Collision Protection**: Existing destination files are never replaced or overwritten.
- ⚙️ **Deterministic Rule Fallbacks**: If local AI is disabled or unavailable, Tidy executes with deterministic extension, date, and project rules without loss of functionality.

---

## Workspace Architecture & Packages

Tidy is structured as a modular Rust workspace comprising six specialized crates and a modern desktop application:

```
Tidy/
├── apps/
│   └── desktop/             # Tauri 2 + React 19 + TypeScript + Tailwind CSS application
├── crates/
│   ├── platform/            # Native OS abstraction, handle safety, identity tracking
│   ├── file-indexer/        # Scoped metadata scanner, SQLite WAL indexer & CLI
│   ├── agent-runtime/       # Local LLM supervision, 39 workflow skills, conversational planner
│   ├── organization/        # Deterministic organization rules (Category, Date, Project)
│   ├── storage/             # Space reclamation, exact duplicate deduplication, build artifact cleaner
│   └── safety/              # Transaction journal, OS Trash integration, approved undo engine
├── native/
│   └── inference/           # Pinned llama.cpp C++ runtime with Apple Metal & CPU acceleration
├── models/                  # Curated model manifests (Qwen 1.7B / 0.6B) & benchmark protocol
├── skills/                  # 39 domain-specific guided workflow skills
└── tests/                   # Strict boundary and contract verification tests
```

---

## Versions & Evolutionary Roadmap (V1 – V7)

Tidy has evolved through seven distinct architectural phases, each delivering isolated packages, strict safety boundaries, and concrete verification gates:

| Version | Milestone Title | Primary Packages Introduced | Key Capabilities & Milestones |
| :---: | :--- | :--- | :--- |
| **V1** | **Architecture & Foundation** | `tidy-platform`, `tidy-file-indexer` | 6-crate workspace layout; explicit read grants (`AuthorizedRoot`); bounded metadata scanner & CLI tool (`tidy-scan`); zero mutation privileges. |
| **V2** | **Persistent Scanning & Desktop Scaffold** | `tidy-desktop`, `tidy-file-indexer` (SQLite) | Tauri 2 desktop app; Tokio background tasks; SQLite WAL incremental indexing; handle-based containment (`openat`, `O_NOFOLLOW`); bounded plain-text extraction & staged SHA-256 deduplication. |
| **V3** | **Local Offline Inference Engine** | `native/inference`, `tidy-agent-runtime` | Pinned `llama.cpp` integration with CMake; Apple Metal GPU acceleration; zero-network runtime operation; child process supervision with timeout/RAM guards. |
| **V4** | **Planning & Storage Intelligence** | `tidy-organization`, `tidy-storage` | Deterministic rule engines (Category, Civil Date, Project); disk reclamation analyzer (large files, verified duplicates, stale installers, `node_modules` / `target` artifacts). |
| **V5** | **Safety System & Atomic Journaling** | `tidy-safety` | 3-stage validation pipeline; single-use cryptographic approval tokens; durable SQLite transaction journal; native OS Trash integration; crash recovery and verified undo. |
| **V6** | **Conversational Assistant & Guided Workflows** | `tidy-agent-runtime`, `tidy-desktop` | Unified continuous "Ask Tidy" chat stream; 39 specialized local workflow skills; live investigation timeline; instant non-AI shortcuts (extension batch renames, text cleanup). |
| **V7** | **Benchmarking, Packaging & Distribution** | Full Workspace & GitHub CI/CD | Self-contained macOS `.app` bundle; cross-platform GitHub Actions CI matrix (macOS, Ubuntu, Windows); automated CD release pipeline publishing packages on every tag. |

👉 For an in-depth breakdown of each version's architectural contracts and verification commands, see [docs/VERSIONS.md](docs/VERSIONS.md).

---

## Continuous Integration & Delivery (CI/CD)

Tidy features an automated CI/CD pipeline powered by GitHub Actions:

- **Continuous Integration (`.github/workflows/ci.yml`)**:
  - **Cross-Platform Core Checks**: Compiles and runs all unit and contract tests across **macOS**, **Ubuntu**, and **Windows**.
  - **Zero-Warning Enforcement**: Enforces `cargo clippy --all-targets --locked --offline -- -D warnings` and `cargo fmt --all -- --check`.
  - **Desktop Build & Verification**: Validates the complete desktop application, TypeScript typechecks, Vite production bundle, native inference worker build, and Tauri macOS packaging.
- **Continuous Delivery (`.github/workflows/cd.yml`)**:
  - Automatically triggered whenever a version tag (e.g. `v0.8.1`, `v1.0.0` – `v7.0.0`) is pushed.
  - Builds the production, self-contained macOS application package (`Tidy.app`).
  - Generates `.tar.gz`, `.zip` distribution bundles, and cryptographic `SHA256SUMS.txt`.
  - Automatically creates a GitHub Release and attaches the packaged release binaries.

---

## Getting Started

### Prerequisites

- **Rust**: 1.90+ (tested with stable)
- **Node.js**: 22+
- **macOS**: Xcode command-line tools (`xcode-select --install`) and CMake (for native inference engine)

### Development Setup

1. **Clone the repository**:
   ```sh
   git clone git@github.com:NicolasLasch/Tidy.git
   cd Tidy
   ```

2. **Install frontend dependencies & build the native inference engine**:
   ```sh
   cd apps/desktop
   npm ci
   npm run worker:build   # Compiles pinned llama.cpp with Apple Metal acceleration
   ```

3. **Launch Tidy in development mode**:
   ```sh
   npm run desktop        # Launches Tauri desktop application with hot reloading
   ```

### Building Self-Contained Release Packages

To build the self-contained macOS desktop bundle:

```sh
cd apps/desktop
npm run package
```

The resulting application is placed at:
```
target/release/bundle/macos/Tidy.app
```
Once packaged, `Tidy.app` launches natively without requiring any local Node or development dependencies.

---

## CLI & Verification Suite

The standalone metadata scanner CLI is available for terminal use:

```sh
cargo run --bin tidy-scan -- --root "$HOME/Downloads"
```

To run the complete verification suite across all workspace crates:

```sh
# Verify formatting
cargo fmt --all -- --check

# Run full test suite (offline)
cargo test --locked --offline

# Run strict Clippy verification
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
```

---

## Local AI Setup (Optional)

Tidy functions fully without AI models using deterministic rules. If you wish to enable the local intelligence assistant:

1. Open **Set up local AI** within the application settings.
2. Select your preferred local model:
   - **Qwen3 1.7B Q4_K_M** (~1.28 GB) — Recommended for balanced reasoning.
   - **Qwen3 0.6B Q4_0** (~0.43 GB) — Ultra-lightweight footprint.
3. Click **Download model**. Downloads are verified against pinned SHA-256 checksums and saved to your application support directory.
4. Model execution runs 100% offline via Apple Metal or multi-threaded CPU. Zero prompts or file data leave your computer.

---

## License

Tidy is licensed under the [MIT License](LICENSE).
