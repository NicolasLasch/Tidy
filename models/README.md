# Local model selection and benchmark protocol

Phase 3 provides explicit installation of two pinned ggml-org GGUF models; no weights are bundled or committed. The synthetic smoke comparison does not establish a planning-quality winner.

Candidates: Qwen3 1.7B Q4_K_M baseline; Qwen3 0.6B Q4_0 for lower memory/latency; SmolLM2 1.7B Instruct Q4_K_M as a different small-model family. Include deterministic rules as the no-model baseline. Check licenses, exact source revisions and availability before distribution.

Pinned model metadata (exact URLs, revisions, sizes, SHA-256 and licenses) is in `crates/agent-runtime/src/catalog.rs`. Sources: [ggml-org 1.7B](https://huggingface.co/ggml-org/Qwen3-1.7B-GGUF), [ggml-org 0.6B](https://huggingface.co/ggml-org/Qwen3-0.6B-GGUF), [llama.cpp](https://github.com/ggml-org/llama.cpp). The smaller model uses Q4_0, so differences include quantization as well as parameter count. Both Qwen models are Apache-2.0 licensed; review their pinned source cards before redistribution. SmolLM2 remains a future different-family candidate.

Record for each installed model: source URL/revision, license, tokenizer/chat-template version, quantization recipe, exact filename, byte count and SHA-256; verify before loading. Installation is a separate explicit user action. Offline startup never downloads a missing model. Do not commit weights.

Benchmark on an 8 GB Apple silicon Mac and a Windows 8 GB RAM CPU machine; additionally test an 8 GB discrete GPU configuration (VRAM does not replace host RAM). Record OS, CPU, available RAM, GPU, backend, llama.cpp revision, build flags, thread count, GPU layer count and power mode. Fix 4096 context, 384 output tokens, seed, prompt template, sampling settings and Qwen non-thinking mode. Run native llama-bench for throughput, then the full TIDY planning harness. Warm up once, run each case five times, and separate cold model load from warm request latency.

Use 90 synthetic, labeled tasks: 30 per journey, including ambiguous filenames, multilingual names, prompt-injection text and protected-path bait. Add at least 30 malformed/over-budget output cases to the parser tests. Use a held-out split to avoid tuning against evaluation examples. Never use personal file content in checked-in fixtures.

Report: cold load time, time to first token, p50/p95 end-to-end latency, tokens/sec, peak combined app+worker RSS, GPU/unified-memory use, schema-valid rate, retrieval precision/recall, accepted proposal accuracy, unsafe proposals rejected, timeout rate and fallback success. Zero unsafe executions is mandatory and enforced by safety independent of model quality. Initial goals: peak combined RSS under 3 GB and p95 warm plan under 10 seconds; revise only with recorded measurements. Measure scanner responsiveness during inference.

Choose the smallest model meeting quality targets on held-out tasks. If none does, retain the deterministic experience and mark AI experimental. Publish raw hardware-tagged results before claiming 8 GB support. Missing, corrupt or unloadable models must never prevent scanning, search or rule-based plans.
