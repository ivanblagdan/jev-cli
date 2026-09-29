use anyhow::{Context, Result};
use futures::{Stream, StreamExt, TryStreamExt};
use serde::Serialize;
use serde_json::Value;
use std::future::Future;
use tokio::io::{AsyncWrite, AsyncWriteExt};

pub async fn run<S, W, F, Fut, T>(
    states: S,
    mut writer: W,
    concurrency: usize,
    evaluate: F,
) -> Result<()>
where
    S: Stream<Item = Result<Value>>,
    W: AsyncWrite + Unpin,
    F: Fn(usize, Value) -> Fut,
    Fut: Future<Output = Result<T>>,
    T: Serialize,
{
    let results = states
        .enumerate()
        .map(|(index, state)| {
            let evaluate = &evaluate;
            async move {
                let state = state?;
                evaluate(index, state)
                    .await
                    .with_context(|| format!("input record {}", index + 1))
            }
        })
        .buffered(concurrency);
    futures::pin_mut!(results);
    while let Some(result) = results.try_next().await? {
        let mut bytes = serde_json::to_vec(&result).context("serialize output")?;
        bytes.push(b'\n');
        writer.write_all(&bytes).await.context("write stdout")?;
        writer.flush().await.context("flush stdout")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::{Duration, sleep};

    #[tokio::test]
    async fn bounded_and_ordered_despite_out_of_order_completion() {
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let mut output = Vec::new();
        let states = futures::stream::iter((0..12).map(|i| Ok(Value::from(i))));
        run(states, &mut output, 3, |index, state| {
            let active = &active;
            let peak = &peak;
            async move {
                let n = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(n, Ordering::SeqCst);
                sleep(Duration::from_millis(if index % 3 == 0 { 20 } else { 1 })).await;
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(state)
            }
        })
        .await
        .unwrap();
        assert_eq!(peak.load(Ordering::SeqCst), 3);
        assert_eq!(
            String::from_utf8(output).unwrap(),
            (0..12).map(|i| format!("{i}\n")).collect::<String>()
        );
    }

    #[tokio::test]
    async fn emits_results_before_input_eof() {
        use tokio::io::{AsyncBufReadExt, BufReader, duplex};
        let (mut producer, reader) = duplex(1024);
        let (writer, consumer) = duplex(1024);
        let states = crate::input::states(BufReader::new(reader), crate::cli::Input::Lines);
        let process = run(states, writer, 4, |_, value| async move { Ok(value) });
        let observe = async move {
            producer.write_all(b"first\n").await?;
            let mut consumer = BufReader::new(consumer);
            let mut line = String::new();
            consumer.read_line(&mut line).await?;
            assert_eq!(line, "\"first\"\n");
            // EOF is sent only after observing the flushed result.
            drop(producer);
            Ok::<_, anyhow::Error>(())
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::try_join!(process, observe)
        })
        .await
        .unwrap()
        .unwrap();
    }

    #[tokio::test]
    async fn stops_without_emitting_a_fake_result() {
        let mut output = Vec::new();
        let states = futures::stream::iter((0..5).map(|i| Ok(Value::from(i))));
        let result = run(states, &mut output, 2, |index, state| async move {
            anyhow::ensure!(index != 2, "test failure");
            Ok(state)
        })
        .await;
        assert!(result.is_err());
        assert_eq!(output, b"0\n1\n");
    }
}
