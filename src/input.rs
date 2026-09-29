use crate::cli::Input;
use anyhow::{Context, Result, ensure};
use futures::stream::{self, Stream};
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt};

/// Read only when polled: the downstream bounded buffer provides backpressure.
pub fn states<R: AsyncBufRead + Unpin>(
    reader: R,
    mode: Input,
) -> impl Stream<Item = Result<Value>> {
    stream::try_unfold(
        (reader, 0usize, false),
        move |(mut reader, index, done)| async move {
            if done {
                return Ok(None);
            }
            let mut text = String::new();
            let streaming = matches!(mode, Input::Lines | Input::Jsonl);
            if streaming {
                if reader.read_line(&mut text).await.context("read stdin")? == 0 {
                    return Ok(None);
                }
                if text.ends_with('\n') {
                    text.pop();
                    if text.ends_with('\r') {
                        text.pop();
                    }
                }
            } else {
                reader
                    .read_to_string(&mut text)
                    .await
                    .context("read stdin")?;
                ensure!(!text.is_empty(), "stdin is empty");
            }
            let state = match mode {
                Input::Text | Input::Lines => Value::String(text),
                Input::Json | Input::Jsonl => {
                    let value: Value = serde_json::from_str(&text)
                        .with_context(|| format!("invalid JSON at input record {}", index + 1))?;
                    ensure!(
                        matches!(value, Value::String(_) | Value::Object(_) | Value::Array(_)),
                        "input record {}: state must be a string, object, or array",
                        index + 1
                    );
                    value
                }
            };
            Ok(Some((state, (reader, index + 1, !streaming))))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    #[tokio::test]
    async fn lines_preserve_empty_records_and_strip_crlf() {
        let values: Vec<_> = states(&b"one\r\n\nthree"[..], Input::Lines)
            .try_collect()
            .await
            .unwrap();
        assert_eq!(
            values,
            vec![Value::from("one"), Value::from(""), Value::from("three")]
        );
    }
    #[tokio::test]
    async fn jsonl_rejects_blank_and_scalar_records() {
        for bytes in [b"{}\n\n".as_slice(), b"42\n", b"null\n", b"true\n"] {
            assert!(
                states(bytes, Input::Jsonl)
                    .try_collect::<Vec<_>>()
                    .await
                    .is_err()
            );
        }
    }
    #[tokio::test]
    async fn json_and_empty_stream() {
        assert_eq!(
            states(&b"{\n\"x\": 1\n}"[..], Input::Json)
                .try_collect::<Vec<_>>()
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            states(&b""[..], Input::Lines)
                .try_collect::<Vec<_>>()
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            states(&b""[..], Input::Text)
                .try_collect::<Vec<_>>()
                .await
                .is_err()
        );
    }
}
