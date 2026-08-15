//! MCP tool registry — the single source of truth for tool metadata, dispatch, and license gating.
//!
//! Design principles:
//! - Each tool is a `ToolDef`: name + description + schema + tier + handler in ONE place.
//!   No more 3-file desync (tools.rs schema vs handlers.rs logic vs mod.rs routing).
//! - License enforcement happens once in `dispatch()`, not per-tool.
//! - Cache invalidation is declarative (`invalidates_evolution` flag), not manual.
//! - Adding a new tool = adding one `ToolDef` and registering it. Nothing else.

use super::McpState;
use crate::license::Tier;
use serde_json::{json, Value};

/// Handler function signature: args + tier + mutable state → result.
/// Every tool handler has this exact signature, enabling uniform dispatch.
/// Public so external crates (sentrux-pro) can define handlers.
pub type ToolHandler = fn(&Value, &Tier, &mut McpState) -> Result<Value, String>;

/// Complete definition of an MCP tool.
/// Schema, handler, and tier requirement are co-located — impossible to desync.
/// Public so external crates (sentrux-pro) can register tools.
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    pub min_tier: Tier,
    pub handler: ToolHandler,
    /// If true, clears `cached_evolution` before execution (snapshot is about to change).
    pub invalidates_evolution: bool,
}

/// Registry holding all tools. Built once at MCP server startup.
pub struct ToolRegistry {
    tools: Vec<ToolDef>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: Vec::with_capacity(24),
        }
    }

    /// Register a tool. Panics on duplicate names (programming error, caught at startup).
    pub fn register(&mut self, tool: ToolDef) {
        debug_assert!(
            !self.tools.iter().any(|t| t.name == tool.name),
            "Duplicate tool registration: {}",
            tool.name
        );
        self.tools.push(tool);
    }

    /// Dispatch a tool call with automatic license enforcement and cache management.
    ///
    /// Order of operations:
    /// 1. Find tool by name (or return error)
    /// 2. Check license tier (or return upgrade message)
    /// 3. Invalidate stale caches if tool modifies snapshot
    /// 4. Execute handler
    pub fn dispatch(
        &self,
        name: &str,
        args: &Value,
        tier: &Tier,
        state: &mut McpState,
    ) -> Result<Value, String> {
        let tool = self
            .tools
            .iter()
            .find(|t| t.name == name)
            .ok_or_else(|| format!("Unknown tool: {name}"))?;

        // License gate
        if !tier.can_access(tool.min_tier) {
            return Err(upgrade_message(name, tool.min_tier));
        }

        // Pre-execution: invalidate stale caches
        if tool.invalidates_evolution {
            state.cached_evolution = None;
        }

        // Record MCP call for telemetry
        crate::app::update_check::record_mcp_call();
        if name == "gate" {
            crate::app::update_check::record_gate_run();
        }

        // Execute
        (tool.handler)(args, tier, state)
    }

    /// Generate the `tools/list` response from registered tools.
    /// All tools are listed regardless of tier — the agent should know they exist
    /// so it can inform the user about upgrade options.
    pub fn definitions(&self) -> Value {
        let defs: Vec<Value> = self
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": t.input_schema
                })
            })
            .collect();
        json!(defs)
    }
}

/// Construct a clear, actionable upgrade message for gated tools.
fn upgrade_message(tool: &str, required: Tier) -> String {
    format!(
        "'{tool}' requires sentrux {required}. \
         Learn more: https://github.com/sentrux/sentrux"
    )
}
