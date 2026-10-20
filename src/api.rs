use crate::{cli::Common, question::Question};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime},
};
use tokio::{
    sync::Mutex,
    time::{Instant, sleep_until},
};

pub struct Api {
    client: reqwest::Client,
    endpoint: String,
    key: String,
    model: String,
    retries: u8,
    cooldown: Mutex<Instant>,
}

#[derive(Deserialize)]
struct Response {
    model: String,
    answers: BTreeMap<String, Value>,
    usage: Usage,
}

#[derive(Deserialize, Serialize)]
pub struct Usage {
    input_tokens: u64,
    output_tokens: u64,
}

#[derive(Serialize)]
pub struct Output {
    input: Value,
    result: Value,
    model: String,
    usage: Usage,
}

impl Api {
    pub fn new(options: &Common, key: String) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(options.timeout.into()))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            endpoint: "https://api.typesafe.ai/v1/systemone".into(),
            key,
            model: options.model.clone(),
            retries: options.retries,
            cooldown: Mutex::new(Instant::now()),
        })
    }

    async fn wait(&self) {
        loop {
            let deadline = *self.cooldown.lock().await;
            if deadline <= Instant::now() {
                return;
            }
            sleep_until(deadline).await;
        }
    }

    pub async fn evaluate(&self, state: Value, question: &Question) -> Result<Output> {
        let mut payload =
            json!({"model": self.model, "state": state, "questions": {"result": question}});
        for attempt in 0..=self.retries {
            self.wait().await;
            let response = self
                .client
                .post(&self.endpoint)
                .bearer_auth(&self.key)
                .json(&payload)
                .send()
                .await
                .context("Jev request failed (transport errors are not retried)")?;
            let status = response.status();
            if (status.as_u16() == 429 || status.is_server_error()) && attempt < self.retries {
                let delay = retry_delay(response.headers().get("retry-after"), attempt)?;
                let deadline = Instant::now()
                    .checked_add(delay)
                    .context("Retry-After is too large")?;
                let mut cooldown = self.cooldown.lock().await;
                *cooldown = (*cooldown).max(deadline);
                continue;
            }
            // Do not print response bodies: they may echo private input.
            ensure!(status.is_success(), "Jev API returned HTTP {status}");
            let mut response: Response = response.json().await.context("malformed Jev response")?;
            ensure!(!response.model.is_empty(), "response model is empty");
            // Moving aliases legitimately resolve to a different concrete model name.
            if pinned(&self.model) {
                ensure!(
                    response.model == self.model,
                    "response model does not match requested pinned model"
                );
            }
            ensure!(response.answers.len() == 1, "expected exactly one answer");
            let answer = response
                .answers
                .remove("result")
                .context("missing result answer")?;
            question
                .validate_answer(&answer)
                .context("invalid Jev answer")?;
            return Ok(Output {
                input: payload["state"].take(),
                result: answer,
                model: response.model,
                usage: response.usage,
            });
        }
        unreachable!("retry loop always executes")
    }
}

fn pinned(model: &str) -> bool {
    model.strip_prefix("jev-").is_some_and(|version| {
        let parts: Vec<_> = version.split('.').collect();
        parts.len() == 3
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()))
    })
}

fn retry_delay(header: Option<&reqwest::header::HeaderValue>, attempt: u8) -> Result<Duration> {
    let Some(header) = header else {
        return Ok(Duration::from_secs(1u64 << attempt));
    };
    let value = header.to_str().context("invalid Retry-After header")?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Ok(Duration::from_secs(seconds));
    }
    Ok(httpdate::parse_http_date(value)
        .context("invalid Retry-After header")?
        .duration_since(SystemTime::now())
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Input;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    async fn server(replies: Vec<(u16, Value)>) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body) in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buf = [0; 4096];
                    let n = socket.read(&mut buf).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buf[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                        assert!(headers.contains("authorization: bearer test-key"));
                        let length: usize = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if bytes.len() >= end + 4 + length {
                            requests.push(
                                serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap(),
                            );
                            break;
                        }
                    }
                }
                let body = body.to_string();
                socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nRetry-After: 0\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
            requests
        });
        (format!("http://{address}"), task)
    }

    fn options() -> Common {
        Common {
            instructions: None,
            question_file: None,
            input: Input::Text,
            concurrency: 4,
            model: "jev-1.13.0".into(),
            timeout: 2,
            retries: 1,
        }
    }

    #[tokio::test]
    async fn all_primitives_match_wire_contract() {
        for (question, answer) in [
            (
                json!({"type":"noul","instructions":"Yes?"}),
                json!({"type":"noul","noul":0.8}),
            ),
            (
                json!({"type":"choice","instructions":"Pick","criteria":{"a":null,"b":"Other"}}),
                json!({"type":"choice","choice":"a","probabilities":{"a":0.8,"b":0.2},"confidence":0.5}),
            ),
            (
                json!({"type":"score","instructions":"Rate","criteria":["Low","High"]}),
                json!({"type":"score","score":0.8,"probabilities":{"0":0.2,"1":0.8},"legend":{"0":"Low","1":"High"},"confidence":0.5}),
            ),
        ] {
            let response = json!({"model":"jev-1.13.0","answers":{"result":answer},"usage":{"input_tokens":10,"output_tokens":2}});
            let (endpoint, task) = server(vec![(200, response)]).await;
            let mut api = Api::new(&options(), "test-key".into()).unwrap();
            api.endpoint = endpoint;
            let parsed: Question = serde_json::from_value(question.clone()).unwrap();
            let result = api
                .evaluate(json!({"data":"example"}), &parsed)
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&result).unwrap(),
                json!({
                    "input": {"data":"example"},
                    "result": answer,
                    "model": "jev-1.13.0",
                    "usage": {"input_tokens":10,"output_tokens":2}
                })
            );
            let requests = task.await.unwrap();
            assert_eq!(
                requests[0],
                json!({"model":"jev-1.13.0","state":{"data":"example"},"questions":{"result":question}})
            );
        }
    }

    #[tokio::test]
    async fn output_joins_each_original_input_without_index_or_answer_fields() {
        let question: Question =
            serde_json::from_value(json!({"type":"noul","instructions":"Yes?"})).unwrap();
        for input in [
            json!("line one\nline two"),
            json!({"message":"Email [[VEIL:example:1]]", "nested":{"input":1,"result":2}}),
            json!([{"message":"first"}, "second"]),
        ] {
            let response = json!({"model":"jev-1.13.0","answers":{"result":{"type":"noul","noul":0.8}},"usage":{"input_tokens":1,"output_tokens":1}});
            let (endpoint, task) = server(vec![(200, response)]).await;
            let mut api = Api::new(&options(), "test-key".into()).unwrap();
            api.endpoint = endpoint;
            let output = api.evaluate(input.clone(), &question).await.unwrap();
            let serialized = serde_json::to_value(output).unwrap();
            assert_eq!(serialized["input"], input);
            assert_eq!(serialized["result"], json!({"type":"noul","noul":0.8}));
            assert!(serialized.get("index").is_none());
            assert!(serialized.get("answer").is_none());
            assert_eq!(task.await.unwrap()[0]["state"], input);
        }
    }

    #[test]
    fn retry_headers_and_pinned_models() {
        use reqwest::header::HeaderValue;
        assert_eq!(retry_delay(None, 3).unwrap(), Duration::from_secs(8));
        assert_eq!(
            retry_delay(Some(&HeaderValue::from_static("12")), 0).unwrap(),
            Duration::from_secs(12)
        );
        assert_eq!(
            retry_delay(
                Some(&HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT")),
                0
            )
            .unwrap(),
            Duration::ZERO
        );
        assert!(retry_delay(Some(&HeaderValue::from_static("invalid")), 0).is_err());
        assert!(pinned("jev-1.13.0"));
        assert!(!pinned("jev-latest"));
        assert!(!pinned("jev-1.13"));
    }

    #[tokio::test]
    async fn retry_budget_is_bounded() {
        let q: Question =
            serde_json::from_value(json!({"type":"noul","instructions":"Yes?"})).unwrap();
        let (endpoint, task) = server(vec![(429, json!({})), (429, json!({}))]).await;
        let mut api = Api::new(&options(), "test-key".into()).unwrap();
        api.endpoint = endpoint;
        assert!(api.evaluate(json!("state"), &q).await.is_err());
        assert_eq!(task.await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn aliases_resolve_but_pinned_model_mismatches_fail() {
        let q: Question =
            serde_json::from_value(json!({"type":"noul","instructions":"Yes?"})).unwrap();
        let response = json!({"model":"jev-1.14.0","answers":{"result":{"type":"noul","noul":0.8}},"usage":{"input_tokens":1,"output_tokens":1}});
        for model in ["jev-latest", "jev-1.13.0"] {
            let (endpoint, task) = server(vec![(200, response.clone())]).await;
            let mut options = options();
            options.model = model.into();
            let mut api = Api::new(&options, "test-key".into()).unwrap();
            api.endpoint = endpoint;
            assert_eq!(
                api.evaluate(json!("state"), &q).await.is_ok(),
                model == "jev-latest"
            );
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn retries_overload_but_rejects_bad_answers_and_auth_errors() {
        let q: Question =
            serde_json::from_value(json!({"type":"noul","instructions":"Yes?"})).unwrap();
        let good = json!({"model":"jev-1.13.0","answers":{"result":{"type":"noul","noul":0.8}},"usage":{"input_tokens":1,"output_tokens":1}});
        for status in [429, 529] {
            let (endpoint, task) = server(vec![(status, json!({})), (200, good.clone())]).await;
            let mut api = Api::new(&options(), "test-key".into()).unwrap();
            api.endpoint = endpoint;
            assert!(api.evaluate(json!("state"), &q).await.is_ok());
            assert_eq!(task.await.unwrap().len(), 2);
        }
        for (status, body) in [
            (401, json!({})),
            (
                200,
                json!({"model":"jev-1.13.0","answers":{"result":{"type":"noul","noul":2}},"usage":{"input_tokens":1,"output_tokens":1}}),
            ),
        ] {
            let (endpoint, task) = server(vec![(status, body)]).await;
            let mut api = Api::new(&options(), "test-key".into()).unwrap();
            api.endpoint = endpoint;
            assert!(api.evaluate(json!("state"), &q).await.is_err());
            task.await.unwrap();
        }
    }
}
