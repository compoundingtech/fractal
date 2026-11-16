//! API-equivalent prices for the models st's harnesses use.
//!
//! A response's cost is what its tokens would cost at the provider's published API list price
//! when st recorded it. It is not an invoice: a subscription seat pays a plan price instead. The
//! table names its revision so every recorded cost says which prices it used, and a model the
//! table does not cover is reported as unpriced instead of costing nothing.
//!
//! Sources, read 2026-10-02: Anthropic's model pricing (cache writes are 1.25× input for the
//! five-minute TTL and 2× for the one-hour TTL) and OpenAI's API pricing page.

/// The prices below. Change it with every edit to [`MODELS`].
pub const PRICING_REVISION: &str = "2026-10-02";

pub const PRICE_TABLE_ID: &str = "st.api-list";

/// Content-addressed as well as dated: changing any price always changes the version.
pub fn price_table_version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| version_for(MODELS))
}

fn version_for(table: &[(&str, Rates)]) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{PRICING_REVISION}:{}",
        hex::encode(Sha256::digest(
            serde_json::to_vec(table).expect("finite price table")
        ))
    )
}

/// US dollars per million tokens of each disjoint bucket.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct Rates {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    /// A request whose whole prompt is longer than this many tokens is priced at the long-context
    /// multipliers, every bucket of it.
    pub long_context: Option<LongContext>,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct LongContext {
    pub above_prompt_tokens: u64,
    pub prompt_multiplier: f64,
    pub output_multiplier: f64,
}

const fn anthropic(input: f64, output: f64, cache_read: f64) -> Rates {
    Rates {
        input,
        output,
        cache_read,
        cache_write_5m: input * 1.25,
        cache_write_1h: input * 2.0,
        long_context: None,
    }
}

const OPENAI_LONG_CONTEXT: Option<LongContext> = Some(LongContext {
    above_prompt_tokens: 272_000,
    prompt_multiplier: 2.0,
    output_multiplier: 1.5,
});

const fn openai(input: f64, cached: f64, cache_write: f64, output: f64) -> Rates {
    Rates {
        input,
        output,
        cache_read: cached,
        cache_write_5m: cache_write,
        cache_write_1h: cache_write,
        long_context: OPENAI_LONG_CONTEXT,
    }
}

/// Every model the table covers, by its canonical API ID.
pub const MODELS: &[(&str, Rates)] = &[
    ("claude-fable-5-1", anthropic(10.0, 50.0, 0.25)),
    ("claude-fable-5", anthropic(10.0, 50.0, 1.0)),
    ("claude-opus-5-5", anthropic(4.0, 20.0, 0.20)),
    ("claude-opus-5", anthropic(5.0, 25.0, 0.50)),
    ("claude-opus-4-8", anthropic(5.0, 25.0, 0.50)),
    ("claude-opus-4-7", anthropic(5.0, 25.0, 0.50)),
    ("claude-opus-4-6", anthropic(5.0, 25.0, 0.50)),
    ("claude-sonnet-5-5", anthropic(2.0, 10.0, 0.20)),
    ("claude-sonnet-5", anthropic(2.0, 10.0, 0.20)),
    ("claude-sonnet-4-6", anthropic(3.0, 15.0, 0.30)),
    ("claude-haiku-4-5", anthropic(1.0, 5.0, 0.10)),
    ("gpt-6.1-sol", openai(2.0, 0.10, 2.50, 10.0)),
    ("gpt-6-sol", openai(2.0, 0.20, 2.50, 10.0)),
    ("gpt-6-astra", openai(10.0, 1.0, 12.50, 50.0)),
    ("gpt-6-luna", openai(0.10, 0.01, 0.125, 0.50)),
];

/// The table's rates for `model`, after removing the decorations harnesses add to an API ID: a
/// `provider/` prefix, a bracketed variant such as `[1m]`, and a trailing date snapshot.
pub fn rates(model: &str) -> Option<&'static Rates> {
    let canonical = canonical_model(model);
    MODELS
        .iter()
        .find(|(id, _)| *id == canonical)
        .map(|(_, rates)| rates)
}

/// The decorations harnesses add to an API ID, removed: a `provider/` prefix, a bracketed
/// variant such as `[1m]`, and a trailing date snapshot. Lowercased and trimmed, so a declared
/// `anthropic/claude-opus-5-5` matches usage that reports `claude-opus-5-5-20260901`.
pub fn canonical_model(model: &str) -> String {
    let mut model = model.trim().to_ascii_lowercase();
    if let Some(index) = model.rfind('/') {
        model = model[index + 1..].to_owned();
    }
    if let Some(index) = model.find('[') {
        model.truncate(index);
    }
    if let Some((stem, date)) = model.rsplit_once('-')
        && date.len() == 8
        && date.bytes().all(|byte| byte.is_ascii_digit())
    {
        model = stem.to_owned();
    }
    model
}

/// The Claude harness's model aliases: each names the newest model of its family.
const CLAUDE_ALIASES: &[&str] = &["opus", "sonnet", "haiku"];

/// Whether usage that reports `live` runs the model a seat declares as `declared`, or `None`
/// when the declaration cannot be compared. Both compare after [`canonical_model`]. A Claude
/// alias (`opus`, `sonnet`, `haiku`, with or without a `[1m]` variant) matches any API ID of
/// its family (`sonnet` matches `claude-sonnet-4-6`). A declaration that is neither an alias
/// nor a model this table prices is not a resolvable API ID, so it never reads as a mismatch.
pub fn declared_model_matches(declared: &str, live: &str) -> Option<bool> {
    let declared = canonical_model(declared);
    let live = canonical_model(live);
    if declared == live {
        return Some(true);
    }
    if CLAUDE_ALIASES.contains(&declared.as_str()) {
        return Some(live.starts_with(&format!("claude-{declared}-")));
    }
    MODELS.iter().any(|(id, _)| *id == declared).then_some(false)
}

/// One response's disjoint token buckets. `cache_write_1h` is the part of `cache_write` written
/// with the one-hour TTL.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cache_write_1h: u64,
}

/// The API-equivalent cost of one response in millionths of a US dollar, or `None` when the
/// table has no price for its model.
pub fn cost_microusd(model: &str, tokens: Tokens) -> Option<u64> {
    let rates = rates(model)?;
    let (prompt_multiplier, output_multiplier) = context_multipliers(rates, tokens);
    let one_hour = tokens.cache_write_1h.min(tokens.cache_write);
    // Keep the original arithmetic and per-response rounding when adding provenance.
    let micro = prompt_multiplier
        * (tokens.input as f64 * rates.input
            + tokens.cache_read as f64 * rates.cache_read
            + (tokens.cache_write - one_hour) as f64 * rates.cache_write_5m
            + one_hour as f64 * rates.cache_write_1h)
        + output_multiplier * tokens.output as f64 * rates.output;
    Some(micro.round() as u64)
}

/// Effective USD-per-million rates, including this response's context multipliers.
pub fn applied_rates(model: &str, tokens: Tokens) -> Option<Rates> {
    let mut rates = *rates(model)?;
    let (prompt_multiplier, output_multiplier) = context_multipliers(&rates, tokens);
    rates.input *= prompt_multiplier;
    rates.cache_read *= prompt_multiplier;
    rates.cache_write_5m *= prompt_multiplier;
    rates.cache_write_1h *= prompt_multiplier;
    rates.output *= output_multiplier;
    rates.long_context = None;
    Some(rates)
}

fn context_multipliers(rates: &Rates, tokens: Tokens) -> (f64, f64) {
    let prompt = tokens
        .input
        .saturating_add(tokens.cache_read)
        .saturating_add(tokens.cache_write);
    match rates.long_context {
        Some(long) if prompt > long.above_prompt_tokens => {
            (long.prompt_multiplier, long.output_multiplier)
        }
        _ => (1.0, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_price_change_changes_the_version_without_relying_on_the_label() {
        let mut changed = MODELS.to_vec();
        changed[0].1.input += 0.01;
        assert_ne!(version_for(&changed), price_table_version());
        assert_eq!(version_for(MODELS), price_table_version());
    }

    #[test]
    fn harness_spellings_resolve_to_the_api_model() {
        for spelling in [
            "claude-opus-5-5",
            "claude-opus-5-5[1m]",
            "anthropic/claude-opus-5-5",
            "claude-opus-5-5-20260901",
            "Claude-Opus-5-5",
        ] {
            assert_eq!(rates(spelling), rates("claude-opus-5-5"), "{spelling}");
        }
        assert!(rates("claude-opus-5-5").is_some());
        assert!(rates("local-model").is_none());
        assert_eq!(cost_microusd("local-model", Tokens::default()), None);
    }

    #[test]
    fn a_declared_model_matches_its_spellings_and_claude_aliases_their_family() {
        assert_eq!(declared_model_matches("anthropic/claude-opus-5-5", "claude-opus-5-5-20260901"), Some(true));
        assert_eq!(declared_model_matches("anthropic/claude-opus-5-5", "gpt-6-sol"), Some(false));
        assert_eq!(declared_model_matches("sonnet", "claude-sonnet-4-6"), Some(true));
        assert_eq!(declared_model_matches("opus[1m]", "claude-opus-5-5"), Some(true));
        assert_eq!(declared_model_matches("haiku", "claude-sonnet-4-6"), Some(false));
        assert_eq!(declared_model_matches("sonnet", "gpt-6-sol"), Some(false));
        // Neither an alias nor a priced API ID: not comparable, never a mismatch.
        assert_eq!(declared_model_matches("local-model", "gpt-6-sol"), None);
        assert_eq!(declared_model_matches("local-model", "local-model"), Some(true));
    }

    #[test]
    fn each_bucket_is_priced_at_its_own_rate() {
        // Opus 5.5: $4 input, $20 output, $0.20 cache read, $5 five-minute and $8 one-hour writes.
        let cost = cost_microusd(
            "claude-opus-5-5",
            Tokens {
                input: 1_000,
                output: 2_000,
                cache_read: 100_000,
                cache_write: 3_000,
                cache_write_1h: 1_000,
            },
        )
        .unwrap();
        assert_eq!(cost, 4_000 + 40_000 + 20_000 + 10_000 + 8_000);
    }

    #[test]
    fn a_long_openai_prompt_is_priced_at_the_long_context_rates() {
        let short = Tokens {
            input: 2_000,
            output: 1_000,
            cache_read: 270_000,
            ..Tokens::default()
        };
        assert_eq!(
            cost_microusd("gpt-6.1-sol", short).unwrap(),
            4_000 + 27_000 + 10_000
        );
        let long = Tokens {
            cache_read: 280_000,
            ..short
        };
        assert_eq!(
            cost_microusd("gpt-6.1-sol", long).unwrap(),
            8_000 + 56_000 + 15_000
        );
    }
}
