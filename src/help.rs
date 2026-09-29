//! Shared help contracts keep the three evaluation commands consistent.

pub const ROOT: &str = r#"QUICK START:
  jev auth set-key
  printf '%s' 'Database connection refused' | jev noul 'Does this indicate an outage?'
  jev noul 'Does this indicate an outage?' --input lines < events.log > decisions.jsonl

CHOOSING A COMMAND:
  choice  Select one of 1–255 labels; returns probabilities and confidence.
  score   Rate against 2–10 ordered levels; returns a fractional zero-based index.
  noul    Ask a yes/no question; returns P(yes) in [0, 1], not a boolean.

AUTHENTICATION:
  Set TYPESAFE_API_KEY for headless use, or run `jev auth set-key` interactively.
  The environment overrides the saved key; an empty value is an error.
  Keys are stored in the OS credential store, never in project files.

COMPOSITION:
  State comes from piped/redirected stdin. stdout contains JSON only;
  diagnostics go to stderr. Use --input text/json for one state, or lines/jsonl
  for a stream. Streaming is bounded and preserves input order.
  Each output contains index (zero-based), model, answer, and usage (token counts).
  One question and one API request per state; no thresholds or filtering.
  On failure output may be partial. Exit codes: 0 success, 1 runtime error,
  2 command-line syntax error. Use `set -o pipefail` in shell pipelines.

MORE HELP:
  jev choice --help    Examples, question-file schema, and full I/O contract
  jev score --help
  jev noul --help
  jev auth --help
  Use -h for concise help; --help for detailed help.

Input is sent to TypeSafe and may incur charges. Only send data you are
 authorized to share. Model judgments are probabilistic, not guarantees."#;

pub const AUTH: &str = r#"EXAMPLES:
  jev auth set-key       Save or replace a key using a masked interactive prompt
  jev auth delete-key    Remove the saved key

CREDENTIAL RESOLUTION:
  TYPESAFE_API_KEY overrides the saved key without changing it. An empty value
  is an error, not a request to fall back to the credential store. For CI or
  headless use, inject this environment variable through your secret manager.
  API keys are not accepted as command-line arguments or stored in project files.

STORAGE:
  macOS: Keychain. Windows: Credential Manager. Linux: Secret Service, requiring
  a user-session D-Bus and an accessible, unlocked credential collection.
  There is no plaintext fallback if the credential store is unavailable.
  The service entry is jev.typesafe.ai (account api-key), separate from
  rust-intent-verify's credentials.

Auth status messages go to stderr; stdout is empty. Exit codes: 0 success,
1 runtime/credential error, 2 command-line syntax error."#;

const IO: &str = r#"INPUT:
  Pipe or redirect state into stdin; interactive terminal input is rejected.
  --input text   Entire stdin is one string, preserving newlines (default).
  --input json   Entire stdin is one JSON string, object, or array.
  --input lines  One string per line; removes LF/CRLF, preserves empty lines.
  --input jsonl  One JSON string, object, or array per line. Blank/malformed
                 lines, numbers, booleans, and null are errors.
  Empty text/json input is an error. An empty lines/jsonl stream yields no
  results, but still requires credentials.

OUTPUT:
  One compact JSON object per state, flushed in input order:
    {"index":0,"model":"jev-1.13.0","answer":{...},"usage":{"input_tokens":296,"output_tokens":20}}
  index is zero-based (0 for a single state); join it back to your input records.
  model is the resolved model; answer is the API answer shown above. usage is
  token usage for the successful request, not cumulative retry billing.
  stdout is JSON only; diagnostics go to stderr. Apply thresholds/filtering
  downstream with tools such as jq; model judgments do not change exit status.

QUESTION FILES:
  --question-file reads one API question object, not a whole API request.
  Its type must match the command. It replaces the positional INSTRUCTIONS
  and cannot be combined with --option or --level. Unknown fields are errors.
  Instructions and descriptions accept strings, objects, or arrays. Choice
  descriptions also accept null. Score criteria are ordered descriptions;
  noul criteria optionally describe "true" and/or "false".

STREAMING AND FAILURES:
  Each record makes a separate request using the same question. --concurrency
  bounds pending records, including results waiting for earlier records.
  Completed results are emitted without waiting for stdin EOF. Individual
  record sizes are not capped. There is no cache, resume, or skipped-record mode.
  On error, processing stops and output may be partial. In-flight requests may
  still incur charges even when their results are not emitted.
  Exit codes: 0 success, 1 runtime/input/API/credential/output error,
  2 command-line syntax error. Broken pipes are errors. Use `set -o pipefail`
  when upstream failures must propagate through a shell pipeline.

AUTHENTICATION AND PRIVACY:
  Set TYPESAFE_API_KEY or run `jev auth set-key` for a masked keychain prompt.
  The environment overrides the saved key; an empty value is an error.
  No plaintext credential fallback. See `jev auth --help` for platform details.
  Input is sent to TypeSafe and may incur charges. Only send authorized data.
  Model judgments are probabilistic, not guarantees."#;

pub fn evaluation(kind: &str) -> String {
    let examples = match kind {
        "choice" => {
            r#"EXAMPLES:
  printf '%s' 'My payment failed' | jev choice 'Which team?' --option billing --option technical
  jev choice 'Which team?' --option billing --option technical --input jsonl < tickets.jsonl
  jev choice --question-file routing.json --input jsonl < tickets.jsonl

QUESTION FILE (routing.json):
  {"type":"choice","instructions":"Which team?","criteria":{"billing":"Payments and refunds","technical":"Bugs and outages"}}
  Define 1–255 unique, nonempty option names. Inline --option supplies labels
  without descriptions; use a file for richer criteria.

ANSWER EXAMPLE:
  {"type":"choice","choice":"billing","probabilities":{"billing":0.8,"technical":0.2},"confidence":0.5}
  choice is the selected label; probabilities cover all options. Confidence
  is a separate measure in [0, 1], not the selected option's probability."#
        }
        "score" => {
            r#"EXAMPLES:
  printf '%s' 'A clear explanation' | jev score 'How clear?' --level unclear --level adequate --level excellent
  jev score 'How clear?' --level unclear --level adequate --level excellent --input jsonl < samples.jsonl
  jev score --question-file clarity.json --input jsonl < samples.jsonl

QUESTION FILE (clarity.json):
  {"type":"score","instructions":"How clear?","criteria":["unclear","adequate","excellent"]}
  Define 2–10 levels in ascending order. The score is a probability-weighted
  zero-based level index, NOT an arbitrary rating or percentage. Three levels
  produce a fractional score in [0, 2]. Confidence is in [0, 1].

ANSWER EXAMPLE:
  {"type":"score","score":1.2,"legend":{"0":"unclear","1":"adequate","2":"excellent"},"probabilities":{"0":0.1,"1":0.6,"2":0.3},"confidence":0.2}"#
        }
        "noul" => {
            r#"EXAMPLES:
  printf '%s' 'Database connection refused' | jev noul 'Does this indicate an outage?'
  jev noul 'Does this indicate an outage?' --input lines < events.log
  jev noul 'Are the labels contradictory?' --input jsonl --concurrency 8 < dataset.jsonl > decisions.jsonl
  jq -c 'select(.answer.noul >= 0.9) | {index, probability: .answer.noul}' decisions.jsonl
  jev noul --question-file urgency.json --input jsonl < tickets.jsonl

QUESTION FILE (urgency.json):
  {"type":"noul","instructions":"Is this urgent?","criteria":{"true":"Explicitly time-sensitive","false":"No urgency expressed"}}
  criteria is optional; instructions is required.

ANSWER EXAMPLE:
  {"type":"noul","noul":0.95}
  noul is P(yes) in [0, 1]. No automatic threshold, boolean conversion,
  or separate confidence field is provided."#
        }
        _ => unreachable!("only evaluation primitives have evaluation help"),
    };
    format!("{examples}\n\n{IO}")
}
