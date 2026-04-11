//! Streaming tool executor — runs concurrency-safe tools in parallel.
//!
//! Design insight from Claude Code: Consecutive safe tools (Read, Grep, Glob, etc.)
//! can execute concurrently. Unsafe tools (Bash, Write, Edit) get exclusive access.
//! The executor partitions a batch of tool calls into groups and runs each group
//! with the appropriate concurrency.

use cisco_code_protocol::ToolResult;
use cisco_code_tools::{ToolContext, ToolRegistry};

/// A pending tool call waiting for execution.
pub struct PendingToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// Result of executing a tool call.
pub struct ToolCallResult {
    pub id: String,
    pub name: String,
    pub result: ToolResult,
}

/// Partitions tool calls into safe/unsafe groups and executes them efficiently.
///
/// Safe tools within a consecutive run are executed in parallel via `futures::future::join_all`.
/// When an unsafe tool is encountered, any pending safe batch is flushed first,
/// then the unsafe tool runs exclusively.
pub async fn execute_tool_batch(
    tools: &ToolRegistry,
    calls: Vec<PendingToolCall>,
    ctx: &ToolContext,
) -> Vec<ToolCallResult> {
    let mut results = Vec::with_capacity(calls.len());

    // Partition into groups of consecutive safe/unsafe calls
    let groups = partition_by_safety(tools, &calls);

    for group in groups {
        match group {
            ToolGroup::Safe(indices) => {
                // Run all safe tools in parallel
                let futures: Vec<_> = indices
                    .iter()
                    .map(|&i| {
                        let call = &calls[i];
                        execute_single(tools, &call.id, &call.name, call.input.clone(), ctx)
                    })
                    .collect();

                let batch_results = futures::future::join_all(futures).await;
                results.extend(batch_results);
            }
            ToolGroup::Unsafe(idx) => {
                // Run unsafe tool exclusively
                let call = &calls[idx];
                let r = execute_single(tools, &call.id, &call.name, call.input.clone(), ctx).await;
                results.push(r);
            }
        }
    }

    results
}

/// Execute a single tool call, wrapping errors into ToolResult.
async fn execute_single(
    tools: &ToolRegistry,
    id: &str,
    name: &str,
    input: serde_json::Value,
    ctx: &ToolContext,
) -> ToolCallResult {
    let result = match tools.execute(name, input, ctx).await {
        Ok(r) => r,
        Err(e) => ToolResult::error(format!("Tool execution failed: {e}")),
    };
    ToolCallResult {
        id: id.to_string(),
        name: name.to_string(),
        result,
    }
}

/// A group of tool calls sharing the same safety classification.
enum ToolGroup {
    /// Indices into the calls vec — all concurrency-safe, can run in parallel.
    Safe(Vec<usize>),
    /// Single index — unsafe tool, must run exclusively.
    Unsafe(usize),
}

/// Partition calls into consecutive groups of safe/unsafe tools.
///
/// Example: [Read, Grep, Bash, Read, Glob] → [Safe([0,1]), Unsafe(2), Safe([3,4])]
fn partition_by_safety(tools: &ToolRegistry, calls: &[PendingToolCall]) -> Vec<ToolGroup> {
    let mut groups = Vec::new();
    let mut safe_batch: Vec<usize> = Vec::new();

    for (i, call) in calls.iter().enumerate() {
        let is_safe = tools
            .get(&call.name)
            .map(|t| t.is_concurrency_safe())
            .unwrap_or(false);

        if is_safe {
            safe_batch.push(i);
        } else {
            // Flush any pending safe batch
            if !safe_batch.is_empty() {
                groups.push(ToolGroup::Safe(std::mem::take(&mut safe_batch)));
            }
            groups.push(ToolGroup::Unsafe(i));
        }
    }

    // Flush trailing safe batch
    if !safe_batch.is_empty() {
        groups.push(ToolGroup::Safe(safe_batch));
    }

    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use cisco_code_tools::ToolContext;

    fn test_ctx() -> ToolContext {
        ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
            progress_tx: None,
        }
    }

    #[test]
    fn test_partition_all_safe() {
        let reg = ToolRegistry::with_builtins().unwrap();
        let calls = vec![
            PendingToolCall { id: "1".into(), name: "Read".into(), input: serde_json::json!({}) },
            PendingToolCall { id: "2".into(), name: "Grep".into(), input: serde_json::json!({}) },
            PendingToolCall { id: "3".into(), name: "Glob".into(), input: serde_json::json!({}) },
        ];
        let groups = partition_by_safety(&reg, &calls);
        assert_eq!(groups.len(), 1);
        match &groups[0] {
            ToolGroup::Safe(indices) => assert_eq!(indices, &[0, 1, 2]),
            _ => panic!("expected Safe group"),
        }
    }

    #[test]
    fn test_partition_mixed() {
        let reg = ToolRegistry::with_builtins().unwrap();
        let calls = vec![
            PendingToolCall { id: "1".into(), name: "Read".into(), input: serde_json::json!({}) },
            PendingToolCall { id: "2".into(), name: "Grep".into(), input: serde_json::json!({}) },
            PendingToolCall { id: "3".into(), name: "Bash".into(), input: serde_json::json!({}) },
            PendingToolCall { id: "4".into(), name: "Read".into(), input: serde_json::json!({}) },
            PendingToolCall { id: "5".into(), name: "Glob".into(), input: serde_json::json!({}) },
        ];
        let groups = partition_by_safety(&reg, &calls);
        assert_eq!(groups.len(), 3);
        match &groups[0] {
            ToolGroup::Safe(indices) => assert_eq!(indices, &[0, 1]),
            _ => panic!("expected Safe"),
        }
        match &groups[1] {
            ToolGroup::Unsafe(i) => assert_eq!(*i, 2),
            _ => panic!("expected Unsafe"),
        }
        match &groups[2] {
            ToolGroup::Safe(indices) => assert_eq!(indices, &[3, 4]),
            _ => panic!("expected Safe"),
        }
    }

    #[test]
    fn test_partition_all_unsafe() {
        let reg = ToolRegistry::with_builtins().unwrap();
        let calls = vec![
            PendingToolCall { id: "1".into(), name: "Bash".into(), input: serde_json::json!({}) },
            PendingToolCall { id: "2".into(), name: "Write".into(), input: serde_json::json!({}) },
        ];
        let groups = partition_by_safety(&reg, &calls);
        assert_eq!(groups.len(), 2);
        match &groups[0] {
            ToolGroup::Unsafe(i) => assert_eq!(*i, 0),
            _ => panic!("expected Unsafe"),
        }
        match &groups[1] {
            ToolGroup::Unsafe(i) => assert_eq!(*i, 1),
            _ => panic!("expected Unsafe"),
        }
    }

    #[tokio::test]
    async fn test_execute_batch_empty() {
        let reg = ToolRegistry::with_builtins().unwrap();
        let results = execute_tool_batch(&reg, vec![], &test_ctx()).await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_execute_batch_unknown_tool() {
        let reg = ToolRegistry::with_builtins().unwrap();
        let calls = vec![
            PendingToolCall { id: "1".into(), name: "NonExistent".into(), input: serde_json::json!({}) },
        ];
        let results = execute_tool_batch(&reg, calls, &test_ctx()).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].result.is_error);
    }
}
