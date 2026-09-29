use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// API descriptions deliberately exclude JSON scalars other than strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Description {
    Text(String),
    Object(serde_json::Map<String, Value>),
    Array(Vec<Value>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoulCriteria {
    #[serde(rename = "true", skip_serializing_if = "Option::is_none")]
    pub yes: Option<Description>,
    #[serde(rename = "false", skip_serializing_if = "Option::is_none")]
    pub no: Option<Description>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Question {
    Choice {
        instructions: Description,
        criteria: BTreeMap<String, Option<Description>>,
    },
    Score {
        instructions: Description,
        criteria: Vec<Description>,
    },
    Noul {
        instructions: Description,
        #[serde(skip_serializing_if = "Option::is_none")]
        criteria: Option<NoulCriteria>,
    },
}

impl Question {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Choice { .. } => "choice",
            Self::Score { .. } => "score",
            Self::Noul { .. } => "noul",
        }
    }

    pub fn validate(&self) -> Result<()> {
        let instructions = match self {
            Self::Choice {
                instructions,
                criteria,
            } => {
                ensure!(
                    (1..=255).contains(&criteria.len()),
                    "choice requires 1–255 options"
                );
                ensure!(
                    criteria.keys().all(|key| !key.trim().is_empty()),
                    "option names must not be empty"
                );
                instructions
            }
            Self::Score {
                instructions,
                criteria,
            } => {
                ensure!(
                    (2..=10).contains(&criteria.len()),
                    "score requires 2–10 ordered levels"
                );
                instructions
            }
            Self::Noul { instructions, .. } => instructions,
        };
        if let Description::Text(text) = instructions {
            ensure!(!text.trim().is_empty(), "instructions must not be empty");
        }
        Ok(())
    }

    pub fn validate_answer(&self, answer: &Value) -> Result<()> {
        ensure!(
            answer["type"].as_str() == Some(self.kind()),
            "answer type does not match question"
        );
        match self {
            Self::Noul { .. } => {
                probability(&answer["noul"])?;
            }
            Self::Choice { criteria, .. } => {
                let choice = answer["choice"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("missing choice"))?;
                ensure!(
                    criteria.contains_key(choice),
                    "answer selected an unknown option"
                );
                distribution(answer, criteria.keys().map(String::as_str))?;
            }
            Self::Score { criteria, .. } => {
                let score = answer["score"]
                    .as_f64()
                    .ok_or_else(|| anyhow::anyhow!("missing score"))?;
                ensure!(
                    (0.0..=(criteria.len() - 1) as f64).contains(&score),
                    "score is outside rubric range"
                );
                let keys: Vec<_> = (0..criteria.len()).map(|i| i.to_string()).collect();
                distribution(answer, keys.iter().map(String::as_str))?;
                let legend = answer["legend"]
                    .as_object()
                    .ok_or_else(|| anyhow::anyhow!("missing legend"))?;
                ensure!(
                    legend.len() == keys.len() && keys.iter().all(|k| legend.contains_key(k)),
                    "legend does not match rubric"
                );
            }
        }
        Ok(())
    }
}

fn probability(value: &Value) -> Result<f64> {
    match value.as_f64() {
        Some(p) if (0.0..=1.0).contains(&p) => Ok(p),
        _ => bail!("expected probability in [0, 1]"),
    }
}

fn distribution<'a>(answer: &Value, keys: impl Iterator<Item = &'a str>) -> Result<()> {
    probability(&answer["confidence"])?;
    let probabilities = answer["probabilities"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("missing probabilities"))?;
    let keys: Vec<_> = keys.collect();
    ensure!(
        probabilities.len() == keys.len(),
        "probability options do not match question"
    );
    let mut sum = 0.0;
    for key in keys {
        sum += probability(
            probabilities
                .get(key)
                .ok_or_else(|| anyhow::anyhow!("missing probability for {key}"))?,
        )?;
    }
    ensure!((sum - 1.0).abs() <= 0.01, "probabilities do not sum to 1");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_wrong_question_shapes() {
        for value in [
            json!({"type":"score","instructions":"Rate","criteria":{"a":null}}),
            json!({"type":"noul","instructions":42}),
            json!({"type":"noul","instructions":"Yes?","typo":true}),
        ] {
            assert!(serde_json::from_value::<Question>(value).is_err());
        }
        let q: Question = serde_json::from_value(
            json!({"type":"score","instructions":"Rate","criteria":["only"]}),
        )
        .unwrap();
        assert!(q.validate().is_err());
    }

    #[test]
    fn validates_answers_against_question() {
        let q: Question = serde_json::from_value(
            json!({"type":"choice","instructions":"Pick","criteria":{"a":null,"b":null}}),
        )
        .unwrap();
        assert!(q.validate_answer(&json!({"type":"choice","choice":"a","probabilities":{"a":0.8,"b":0.2},"confidence":0.5})).is_ok());
        assert!(q.validate_answer(&json!({"type":"choice","choice":"c","probabilities":{"a":0.8,"b":0.2},"confidence":0.5})).is_err());
        assert!(q.validate_answer(&json!({"type":"choice","choice":"a","probabilities":{"a":0.8,"c":0.2},"confidence":0.5})).is_err());
    }
}
