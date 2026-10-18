#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Multi-model reasoning effort control level across OpenAI, Anthropic, and local models.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    #[default]
    None,
    Low,
    Medium,
    High,
}

impl ReasoningEffort {
    /// Return the canonical string identifier for the reasoning effort.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// Check if reasoning is disabled.
    pub const fn is_none(self) -> bool {
        matches!(self, Self::None)
    }

    /// Return the corresponding Anthropic extended thinking token budget.
    pub const fn anthropic_thinking_budget(self) -> Option<u32> {
        match self {
            Self::None => None,
            Self::Low => Some(1024),
            Self::Medium => Some(4096),
            Self::High => Some(16384),
        }
    }

    /// Return the corresponding OpenAI reasoning_effort string parameter.
    pub const fn openai_reasoning_effort(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Low => Some("low"),
            Self::Medium => Some("medium"),
            Self::High => Some("high"),
        }
    }
}

impl fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ReasoningEffort {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" | "off" | "disabled" | "0" => Ok(Self::None),
            "low" | "fast" | "1" => Ok(Self::Low),
            "medium" | "balanced" | "2" | "default" => Ok(Self::Medium),
            "high" | "deep" | "extended" | "3" => Ok(Self::High),
            other => Err(format!(
                "invalid reasoning effort '{other}', expected one of: none, low, medium, high"
            )),
        }
    }
}

/// Prompt cache TTL strategy for optimizing KV-cache hits.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptCacheStrategy {
    #[default]
    Auto,
    Interactive,
    Subagent,
    Disabled,
}

impl PromptCacheStrategy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Interactive => "interactive",
            Self::Subagent => "subagent",
            Self::Disabled => "disabled",
        }
    }
}

impl fmt::Display for PromptCacheStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PromptCacheStrategy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "interactive" | "1h" => Ok(Self::Interactive),
            "subagent" | "cron" | "5m" => Ok(Self::Subagent),
            "disabled" | "off" | "none" => Ok(Self::Disabled),
            other => Err(format!(
                "invalid prompt cache strategy '{other}', expected one of: auto, interactive, subagent, disabled"
            )),
        }
    }
}

/// Prompt cache policy configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PromptCachePolicy {
    pub strategy: PromptCacheStrategy,
    pub preserve_static_prefix: bool,
    pub max_prefix_tokens: u32,
}

impl Default for PromptCachePolicy {
    fn default() -> Self {
        Self {
            strategy: PromptCacheStrategy::Auto,
            preserve_static_prefix: true,
            max_prefix_tokens: 32768,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reasoning_effort_conversions() {
        assert_eq!(ReasoningEffort::None.as_str(), "none");
        assert_eq!(ReasoningEffort::Low.as_str(), "low");
        assert_eq!(ReasoningEffort::Medium.as_str(), "medium");
        assert_eq!(ReasoningEffort::High.as_str(), "high");

        assert_eq!(
            ReasoningEffort::from_str("none").unwrap(),
            ReasoningEffort::None
        );
        assert_eq!(
            ReasoningEffort::from_str("low").unwrap(),
            ReasoningEffort::Low
        );
        assert_eq!(
            ReasoningEffort::from_str("medium").unwrap(),
            ReasoningEffort::Medium
        );
        assert_eq!(
            ReasoningEffort::from_str("high").unwrap(),
            ReasoningEffort::High
        );

        assert_eq!(ReasoningEffort::None.anthropic_thinking_budget(), None);
        assert_eq!(ReasoningEffort::Low.anthropic_thinking_budget(), Some(1024));
        assert_eq!(
            ReasoningEffort::Medium.anthropic_thinking_budget(),
            Some(4096)
        );
        assert_eq!(
            ReasoningEffort::High.anthropic_thinking_budget(),
            Some(16384)
        );

        assert_eq!(ReasoningEffort::None.openai_reasoning_effort(), None);
        assert_eq!(ReasoningEffort::Low.openai_reasoning_effort(), Some("low"));
        assert_eq!(
            ReasoningEffort::Medium.openai_reasoning_effort(),
            Some("medium")
        );
        assert_eq!(
            ReasoningEffort::High.openai_reasoning_effort(),
            Some("high")
        );
    }

    #[test]
    fn test_prompt_cache_strategy_parsing() {
        assert_eq!(
            PromptCacheStrategy::from_str("auto").unwrap(),
            PromptCacheStrategy::Auto
        );
        assert_eq!(
            PromptCacheStrategy::from_str("interactive").unwrap(),
            PromptCacheStrategy::Interactive
        );
        assert_eq!(
            PromptCacheStrategy::from_str("subagent").unwrap(),
            PromptCacheStrategy::Subagent
        );
        assert_eq!(
            PromptCacheStrategy::from_str("disabled").unwrap(),
            PromptCacheStrategy::Disabled
        );
    }
}
