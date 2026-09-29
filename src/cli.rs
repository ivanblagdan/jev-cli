use crate::question::{Description, Question};
use anyhow::{Context, Result, ensure};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Parser)]
#[command(
    name = "jev",
    version,
    about = "Structured decisions with TypeSafe's Jev model",
    after_help = "Reads state from stdin; writes one JSON result per state. Input is sent to TypeSafe and may incur charges. Use --help for examples and details.",
    after_long_help = crate::help::ROOT
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Select an option and return its probability distribution.
    #[command(after_long_help = crate::help::evaluation("choice"))]
    Choice {
        #[command(flatten)]
        common: Common,
        /// Repeat to define options without descriptions; use --question-file for richer criteria.
        #[arg(long, conflicts_with = "question_file")]
        option: Vec<String>,
    },
    /// Rate against 2–10 levels; score is a weighted zero-based level index.
    #[command(after_long_help = crate::help::evaluation("score"))]
    Score {
        #[command(flatten)]
        common: Common,
        /// Repeat in ascending order.
        #[arg(long, conflicts_with = "question_file")]
        level: Vec<String>,
    },
    /// Return the probability that a yes/no question is true.
    #[command(after_long_help = crate::help::evaluation("noul"))]
    Noul {
        #[command(flatten)]
        common: Common,
    },
    /// Manage the key in the OS credential store.
    #[command(after_long_help = crate::help::AUTH)]
    Auth {
        #[command(subcommand)]
        command: Auth,
    },
}

#[derive(Subcommand)]
pub enum Auth {
    /// Save or replace the API key using a masked interactive prompt.
    #[command(
        after_long_help = "Requires an interactive terminal and an accessible OS credential store. No key argument is accepted. Saves to service jev.typesafe.ai, account api-key. TYPESAFE_API_KEY still overrides the saved key. Status goes to stderr; stdout is empty. Exit codes: 0 success, 1 runtime error, 2 syntax error. See `jev auth --help` for storage and headless setup."
    )]
    SetKey,
    /// Remove the saved API key from the OS credential store.
    #[command(
        after_long_help = "Removes only the saved jev key; does not unset TYPESAFE_API_KEY or modify other applications' credentials. A missing key or unavailable store is an error. Status goes to stderr; stdout is empty. Exit codes: 0 success, 1 runtime error, 2 syntax error. See `jev auth --help` for details."
    )]
    DeleteKey,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Input {
    Text,
    Json,
    Lines,
    Jsonl,
}

#[derive(Args)]
pub struct Common {
    /// The question to ask of every input state.
    ///
    /// Required unless --question-file is supplied; the two are mutually exclusive.
    #[arg(
        required_unless_present = "question_file",
        conflicts_with = "question_file"
    )]
    pub instructions: Option<String>,
    /// A single API question object, including type, instructions, and criteria.
    #[arg(long)]
    pub question_file: Option<PathBuf>,
    /// text/json read all stdin; lines/jsonl stream one state per line.
    #[arg(long, value_enum, default_value = "text")]
    pub input: Input,
    /// Maximum pending records, including completed results waiting for output order.
    #[arg(long, default_value = "4", value_parser = clap::value_parser!(u16).range(1..))]
    pub concurrency: u16,
    /// Model ID or alias; output records the resolved model.
    ///
    /// Defaults to a pinned version for reproducibility. Moving aliases such as
    /// jev-latest are accepted. A pinned model mismatch is an error; there is no
    /// automatic fallback to another model.
    #[arg(long, default_value = "jev-1.13.0")]
    pub model: String,
    /// Per HTTP attempt timeout in seconds.
    ///
    /// Must be positive. Excludes retry waits; not a deadline for the whole stream.
    #[arg(long, default_value = "60", value_parser = clap::value_parser!(u32).range(1..))]
    pub timeout: u32,
    /// Retries for HTTP 429 and 5xx; transport errors are not retried.
    ///
    /// Additional attempts, from 0 (disabled) to 10. Honors Retry-After or uses
    /// exponential backoff. A shared cooldown pauses new attempts across workers.
    /// Authentication, validation, and malformed-response errors are not retried.
    #[arg(long, default_value = "3", value_parser = clap::value_parser!(u8).range(0..=10))]
    pub retries: u8,
}

impl Command {
    pub fn evaluation(self) -> Result<(Common, Question)> {
        let (common, kind, entries) = match self {
            Self::Choice { common, option } => (common, "choice", option),
            Self::Score { common, level } => (common, "score", level),
            Self::Noul { common } => (common, "noul", vec![]),
            Self::Auth { .. } => unreachable!("auth handled before evaluation"),
        };
        ensure!(!common.model.trim().is_empty(), "model must not be empty");
        let question = if let Some(path) = &common.question_file {
            let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
            serde_json::from_slice::<Question>(&bytes)
                .with_context(|| format!("parse {}", path.display()))?
        } else {
            let instructions = Description::Text(
                common
                    .instructions
                    .clone()
                    .context("missing instructions")?,
            );
            match kind {
                "choice" => {
                    let mut criteria = BTreeMap::new();
                    for option in entries {
                        ensure!(
                            !criteria.contains_key(&option),
                            "duplicate option: {option}"
                        );
                        criteria.insert(option, None);
                    }
                    Question::Choice {
                        instructions,
                        criteria,
                    }
                }
                "score" => Question::Score {
                    instructions,
                    criteria: entries.into_iter().map(Description::Text).collect(),
                },
                _ => Question::Noul {
                    instructions,
                    criteria: None,
                },
            }
        };
        ensure!(
            question.kind() == kind,
            "question file type must match {kind} command"
        );
        question.validate()?;
        Ok((common, question))
    }
}
