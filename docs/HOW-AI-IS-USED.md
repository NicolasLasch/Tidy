# How AI is used in Tidy

**Short version:** a deterministic engine understands most requests instantly; a small local model helps with the rest. The model never touches files.

## Principles

1. **Local first.** Inference runs in a pinned llama.cpp worker on your Mac (Metal or CPU). No prompts or file names leave the machine. The only network action is the explicit *Download model* button.
2. **Propose, don't act.** Model output is *data*: a command sentence, matching rules, or an answer. It is parsed by strict code, validated, and turned into the same reviewed plan as any other request. Execution belongs to the safety engine.
3. **Small models need structure.** Instead of asking a 1–4B model to plan tool calls in free-form JSON (which failed often), Tidy uses AI where small models are reliable — *rewriting* and *classifying* — and code where precision matters.
4. **Untrusted text stays untrusted.** File names, excerpts and paths are treated as data; the worker strips control tokens and prompts say so.

## Where the model runs

| Situation | What happens | Model output is… |
|---|---|---|
| Request the engine understands (most) | No model. ~1–75 ms. | — |
| Engine finds nothing | **Restate:** the model rewrites the request as one plain command from a fixed list of forms, using the real folder names as hints (fixes typos like “life and hell” → `liveandhell-template-1.21.11`). The command is fed back through the same engine. | one line of text |
| “Put my invoices in Finance, photos by year…” | **Build folders:** the model translates the wish into ≤8 matching rules under a JSON *grammar* (llama.cpp GBNF), which are applied deterministically to every indexed file; the result is a reviewable move plan. | grammar-constrained JSON |
| “Where is…?” / questions | **Answer:** exact index statistics + closest matches + the largest folders go into a prompt; the model answers in plain language. | text shown to you |

The chat header shows which model is active, and replies note when a model helped (“How I got this → AI model”).

## Choosing and adding models

The catalog ships pinned models (Qwen3 4B Instruct 2507 recommended; 1.7B and 0.6B for small machines) with revision, size and SHA-256. In **AI → Add your own model**, paste a Hugging Face `.gguf` link: Tidy reads the commit, size and checksum from Hugging Face's API (metadata only), pins them, and the normal verified download installs it. Single-file, ungated, ChatML-format models (Qwen and many fine-tunes) work best; larger models understand more but are slower.

## Why a deterministic engine at all?

Reliability and trust. A file organizer that sometimes “hallucinates” a plan is unusable. Deterministic parsing gives predictable, testable behaviour (see `intent.rs` tests and the end-to-end suites) and millisecond latency, and it makes the model an optional upgrade instead of a requirement.

## Ideas to extend

Semantic search over file *contents* (embeddings), image understanding for photo grouping, learning a user's own naming conventions, and evaluating more models on a small benchmark of real requests — see [IDEAS.md](IDEAS.md).
