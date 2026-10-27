//! Cards.swift's models: the sessions Lantern started with its card tools (engine `cards`), what
//! they ask of you, their progress and their result. Kept after the agent exits.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CardLaunch {
    pub id: String,
    pub kind: String,
    pub cwd: String,
    pub title: String,
    /// The agent running it, once Lantern has seen it start (it stays after the agent exits).
    pub agent_id: Option<String>,
    /// The agent has Lantern's tools (its MCP server started).
    pub tools: bool,
    pub created_at: f64,
    pub ended_at: Option<f64>,
    pub status: Option<String>,
    pub steps: Vec<Step>,
    pub cards: Vec<Card>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Step {
    pub id: String,
    pub title: String,
    pub done: bool,
}

/// A question, a step handed to you, or a result.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Card {
    pub id: String,
    /// ask | handOff | result
    pub kind: String,
    /// waiting | answered | delivered | unconfirmed | expired | stopped | cancelled | shown
    pub state: String,
    pub revision: i64,
    pub created_at: f64,
    pub resolved_at: Option<f64>,
    pub answer: Option<CardAnswer>,
    pub question: Option<String>,
    pub details: Option<String>,
    pub options: Option<Vec<String>>,
    pub reason: Option<String>,
    pub step: Option<String>,
    pub why: Option<String>,
    pub instructions: Option<Vec<String>>,
    pub url: Option<String>,
    pub summary: Option<String>,
    pub outcome: Option<String>,
    pub files: Option<Vec<CardFile>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CardFile {
    pub path: String,
    pub name: String,
    pub folder: bool,
}

/// Your answer: one of the options, your own words, or whether you did the step.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CardAnswer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
}

impl Card {
    pub fn is_waiting(&self) -> bool {
        self.state == "waiting"
    }
}

impl CardLaunch {
    pub fn waiting(&self) -> impl Iterator<Item = &Card> {
        self.cards.iter().filter(|card| card.is_waiting())
    }
}
