//! Cost of usage records: as reported by the agent, or estimated from tokens
//! with built-in catalog prices (Codex reports tokens only).

use serde::{Deserialize, Serialize};

use crate::store::UsageRecord;

/// USD per million tokens.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelPrice {
    pub input: f64,
    pub cached_input: f64,
    pub output: f64,
}

/// Standard API list prices (USD per million tokens: input, cached input,
/// output) for models whose agent reports tokens but no cost. Source:
/// models.dev's OpenAI catalog; base-context tier.
const CATALOG: &[(&str, f64, f64, f64)] = &[
    ("gpt-5", 1.25, 0.125, 10.0),
    ("gpt-5-mini", 0.25, 0.025, 2.0),
    ("gpt-5-nano", 0.05, 0.005, 0.4),
    ("gpt-5-pro", 15.0, 15.0, 120.0),
    ("gpt-5.1", 1.25, 0.125, 10.0),
    ("gpt-5.2", 1.75, 0.175, 14.0),
    ("gpt-5.2-pro", 21.0, 21.0, 168.0),
    ("gpt-5.3-codex", 1.75, 0.175, 14.0),
    ("gpt-5.3-codex-spark", 1.75, 0.175, 14.0),
    ("gpt-5.4", 2.5, 0.25, 15.0),
    ("gpt-5.4-mini", 0.75, 0.075, 4.5),
    ("gpt-5.4-nano", 0.2, 0.02, 1.25),
    ("gpt-5.4-pro", 30.0, 30.0, 180.0),
    ("gpt-5.5", 5.0, 0.5, 30.0),
    ("gpt-5.5-pro", 30.0, 30.0, 180.0),
    ("gpt-5.6", 4.0, 0.4, 20.0),
    ("gpt-5.6-luna", 0.2, 0.02, 1.2),
    ("gpt-5.6-sol", 4.0, 0.4, 20.0),
    ("gpt-5.6-terra", 2.0, 0.2, 12.0),
    ("gpt-6-astra", 10.0, 1.0, 50.0),
    ("gpt-6-luna", 0.1, 0.01, 0.5),
    ("gpt-6-sol", 2.0, 0.2, 10.0),
    ("gpt-6.1-sol", 2.0, 0.1, 10.0),
    ("gpt-6.1-terra", 1.0, 0.05, 5.0),
    ("o3", 2.0, 0.5, 8.0),
    ("o3-mini", 1.1, 0.55, 4.4),
    ("o3-pro", 20.0, 20.0, 80.0),
    ("o4-mini", 1.1, 0.275, 4.4),
];

/// Catalog price for a model value. Accepts an effort suffix
/// (`gpt-6-astra[high]`, `gpt-6-astra/high`) or a provider prefix (`openai/gpt-6-sol`).
pub fn catalog_price(model: &str) -> Option<ModelPrice> {
    let base = model.split('[').next().unwrap_or(model).trim().to_ascii_lowercase();
    let find = |id: &str| {
        CATALOG
            .iter()
            .find(|(m, ..)| *m == id)
            .map(|&(_, input, cached_input, output)| ModelPrice { input, cached_input, output })
    };
    find(&base)
        .or_else(|| base.rsplit('/').next().and_then(find))
        .or_else(|| base.split('/').next().and_then(find))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PricedUsage {
    #[serde(flatten)]
    pub record: UsageRecord,
    /// Reported or estimated cost; `None` when neither is possible.
    pub cost: Option<f64>,
    pub estimated: bool,
}

pub fn price(record: UsageRecord) -> PricedUsage {
    if let Some(cost) = record.cost_usd {
        return PricedUsage { record, cost: Some(cost), estimated: false };
    }
    let estimate = catalog_price(&record.model).map(|p| {
        (record.input_tokens as f64 * p.input
            + record.cached_tokens as f64 * p.cached_input
            + record.output_tokens as f64 * p.output)
            / 1_000_000.0
    });
    PricedUsage { cost: estimate, estimated: estimate.is_some(), record }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(model: &str, cost: Option<f64>) -> UsageRecord {
        UsageRecord {
            session_id: "s".into(),
            workspace_id: "w".into(),
            repo_id: "r".into(),
            agent: "codex".into(),
            model: model.into(),
            ts: 0,
            input_tokens: 1_000_000,
            cached_tokens: 2_000_000,
            output_tokens: 100_000,
            cost_usd: cost,
        }
    }

    #[test]
    fn reported_cost_wins_and_estimates_use_catalog_prices() {
        assert_eq!(price(record("gpt-6-sol", Some(0.3))).cost, Some(0.3));
        // 1M input * $2 + 2M cached * $0.2 + 0.1M output * $10
        for model in ["gpt-6-sol", "gpt-6-sol[high]", "gpt-6-sol/high", "openai/gpt-6-sol"] {
            let p = price(record(model, None));
            assert!(p.estimated, "{model}");
            assert!((p.cost.unwrap() - (2.0 + 0.4 + 1.0)).abs() < 1e-9, "{model}");
        }
        let unknown = price(record("other", None));
        assert_eq!((unknown.cost, unknown.estimated), (None, false));
    }
}
