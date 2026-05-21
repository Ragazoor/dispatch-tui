#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageCategory {
    Keybinding,
    McpTool,
}

impl UsageCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            UsageCategory::Keybinding => "keybinding",
            UsageCategory::McpTool => "mcp_tool",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "keybinding" => Some(Self::Keybinding),
            "mcp_tool" => Some(Self::McpTool),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageActor {
    Human,
    Agent,
}

impl UsageActor {
    pub fn as_str(self) -> &'static str {
        match self {
            UsageActor::Human => "human",
            UsageActor::Agent => "agent",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "human" => Some(Self::Human),
            "agent" => Some(Self::Agent),
            _ => None,
        }
    }
}

/// An event to record in the usage_events table.
#[derive(Debug, Clone)]
pub struct UsageEvent {
    pub category: UsageCategory,
    /// Snake-case name of the action, e.g. "dispatch_task", "create_task".
    pub action: String,
    /// Key char ('d') for keybindings, tool name for MCP tools. None if not applicable.
    pub detail: Option<String>,
    pub actor: UsageActor,
}

/// Aggregated usage row returned by query_usage.
#[derive(Debug, Clone)]
pub struct UsageSummary {
    pub category: String,
    pub action: String,
    pub detail: Option<String>,
    pub actor: String,
    pub count: i64,
    pub last_used: chrono::DateTime<chrono::Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_category_roundtrip() {
        for (cat, s) in [
            (UsageCategory::Keybinding, "keybinding"),
            (UsageCategory::McpTool, "mcp_tool"),
        ] {
            assert_eq!(cat.as_str(), s);
            assert_eq!(UsageCategory::parse(s), Some(cat));
        }
    }

    #[test]
    fn usage_actor_roundtrip() {
        for (actor, s) in [
            (UsageActor::Human, "human"),
            (UsageActor::Agent, "agent"),
        ] {
            assert_eq!(actor.as_str(), s);
            assert_eq!(UsageActor::parse(s), Some(actor));
        }
    }
}
