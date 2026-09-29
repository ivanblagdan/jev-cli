# Jev CLI

A small, composable Rust CLI for [TypeSafe's Jev model](https://docs.typesafe.ai/introduction): **choice**, **score**, and **noul**. Read state from stdin, ask one typed question per state, write JSON to stdout. No application-specific policies or thresholds.

**Input is sent to TypeSafe and may incur API charges.** Model judgments are probabilistic, not guarantees. Only send data you are authorized to share with the service.

## Install and authenticate

```sh
cargo install --path . --locked
jev auth set-key
```

Requires Rust 1.88+. `auth set-key` uses a masked prompt and saves the key in the OS credential store: macOS Keychain, Windows Credential Manager, or Linux Secret Service. Linux requires a running user-session D-Bus and an unlocked credential collection. There is no plaintext fallback. `jev auth delete-key` removes the saved key.

For CI/headless use, supply `TYPESAFE_API_KEY` through your secret manager/environment. It overrides the saved key; an empty value is an error. Keys are not accepted as command-line arguments or stored in project files. Credentials use a separate `jev.typesafe.ai` keychain entry; they do not read or modify rust-intent-verify's entry.

## Agent skill

The optional [`jev-cli` skill](skills/jev-cli/SKILL.md) teaches coding agents when and how to use the CLI: choosing primitives, writing questions, streaming records, and interpreting results safely.

Install through the external [skills installer](https://github.com/vercel-labs/skills) (requires Node.js/npm):

```sh
npx skills add ivanblagdan/jev-cli --skill jev-cli
```

The installer handles harness selection and installation locations. Add `--global` for a user-wide install or `--agent <agent-name>` to select a supported harness explicitly. To install from a local checkout instead:

```sh
npx skills add . --skill jev-cli
```

The skill directory and skill name are both `jev-cli`; the executable remains `jev`. Installing the skill does **not** install the binary or configure API credentials—complete the setup above separately. The CLI itself contains no skill installer or harness-specific integration.

## Three commands

```sh
printf '%s' 'My payment failed' | jev choice 'Which team should handle this?' \
  --option billing --option technical --option sales

printf '%s' 'The answer is difficult to follow' | jev score 'How clear is this explanation?' \
  --level unclear --level adequate --level excellent

printf '%s' 'Database connection refused' | jev noul 'Does this indicate a service outage?'
```

- **Choice:** 1–255 named options; returns the selected option, probabilities, and confidence. Duplicate/empty option names are rejected.
- **Score:** 2–10 ordered level descriptions; returns a probability-weighted **zero-based level index**, legend, probabilities, and confidence. Three levels produce scores in [0, 2], including fractional scores.
- **Noul:** returns `noul`, the probability of yes in [0, 1]. It does not apply a threshold or return a separate confidence.

Every command accepts concise `-h` and detailed `--help`. Detailed help includes examples, question-file schemas, authentication, output fields, streaming failure behavior, and exit codes; no README is needed to get started. Use `jev auth --help` for credential setup. A positional question is required unless using `--question-file`.

## Input and streaming

| `--input` | Behavior |
| --- | --- |
| `text` (default) | Entire stdin is one string, including newlines. |
| `json` | Entire stdin is one JSON string, object, or array. |
| `lines` | One plain-text state per line; removes LF/CRLF delimiters. Empty lines are empty-string states. |
| `jsonl` | One JSON string, object, or array per line. Blank/malformed lines and scalar numbers, booleans, or null are errors. |

```sh
jev noul 'Does this event indicate data loss?' --input lines < events.log

jev noul 'Does this record contain contradictory labels?' \
  --input jsonl --concurrency 8 < dataset.jsonl > decisions.jsonl

# Leave selection policy to ordinary tools.
jq -c 'select(.answer.noul >= 0.9) | {index, probability: .answer.noul}' decisions.jsonl
```

Input must be piped or redirected, not an interactive terminal. Empty single-state input is an error. An empty stream yields no results (credentials are still checked).

Streaming sends **one request per record**, using the same question. Up to `--concurrency` records are pending, including completed results waiting for earlier ones. Output remains in input order, with bounded buffering and backpressure. Each result is flushed immediately when it can be emitted in order. Memory scales with pending record sizes, not total dataset size; individual record size is not capped locally.

This is not server-side dataset batching. TypeSafe also supports several questions against one shared state in a single request; this CLI deliberately exposes one primitive/question per invocation.

## Output contract

One compact JSON object per input state, on one line:

```json
{"index":0,"model":"jev-1.13.0","answer":{"type":"noul","noul":0.95},"usage":{"input_tokens":296,"output_tokens":20}}
```

- `index` is the zero-based input record index (always 0 for single-state input). It allows joining results back to inputs without copying potentially sensitive records into output.
- `answer` retains the API's answer object, including probabilities/confidence where provided.
- `model` is the actual model reported by the service.
- `usage` contains input/output token counts for the successful request, not cumulative retry billing.

The CLI checks response shape, answer type, expected options/levels, score/probability ranges, and pinned model identity before emitting a result. No model judgment changes the exit code; apply acceptance policies downstream.

All diagnostics and auth status messages go to **stderr**. Exit codes: **0** success, **1** runtime/input/API/credential/output error, **2** command-line syntax error. Use `set -o pipefail` in shell pipelines when upstream failures must propagate.

On error, processing stops with a nonzero exit. Already emitted results remain valid, but the output is **partial**, not transactional. Requests already in flight may have reached the service and incurred charges, even if their results are never emitted. There is no automatic resume, cache, or skipped-record fallback. Broken output pipes are errors too.

## Structured questions

Use a file containing one [API question object](https://docs.typesafe.ai/api), not a whole request. The file's type must match the command. `--question-file` cannot be combined with the positional question, `--option`, or `--level`.

`routing.json`:

```json
{
  "type": "choice",
  "instructions": "Which team should handle this?",
  "criteria": {
    "billing": "Payments, invoices, refunds",
    "technical": "Bugs, outages, integrations",
    "sales": "Pricing and upgrades"
  }
}
```

```sh
jev choice --question-file routing.json --input jsonl < tickets.jsonl
```

Instructions and descriptions can be strings, objects, or arrays, matching the API. Choice descriptions can also be null. Score criteria are an ordered array. Noul optionally accepts `criteria` with `"true"` and `"false"` descriptions. Unknown question fields are rejected rather than silently ignored.

## Request controls

| Flag | Default | Meaning |
| --- | --- | --- |
| `--model` | `jev-1.13.0` | Pinned model for reproducibility. Moving aliases such as `jev-latest` are accepted; output records the resolved model. |
| `--concurrency` | `4` | Maximum pending records; use 1 for sequential processing. |
| `--timeout` | `60` | Timeout in seconds per HTTP attempt, not for the entire stream or retry wait. |
| `--retries` | `3` | Additional attempts for HTTP 429/5xx, including 529; 0 disables retries, maximum 10. |

Requests use `POST https://api.typesafe.ai/v1/systemone`, bearer authentication, and TLS. Redirects are disabled. Rate-limit/server-error retries honor `Retry-After` (seconds or HTTP date), otherwise use exponential backoff. A shared cooldown pauses new attempts across all workers; requests already in flight are not recalled. Transport errors/timeouts are not retried because the server may already have processed the request. Authentication, validation, malformed-response, and local input errors are not retried. There is no fallback to another model. HTTP error bodies are not printed because they may echo private input.

## Development

```sh
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --check
```

Tests use local mock HTTP servers and synthetic data; no API key or paid requests are needed. The implementation separates CLI/question validation, input framing, credential handling, HTTP transport, and bounded ordered execution in `src/`.
