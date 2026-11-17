//! 计划者（第 4/8 节）：L3 且确需拆分时生成子任务 DAG，后端校验无环/依赖/归属/范围。
//! 计划无效时降为单执行者（困难任务可由一个强执行者完成），不自由启动嵌套委派。

use super::contracts::{AcceptanceCriterion, NodeKind, TaskNode};
use super::executor::{extract_json_envelope, AgentExecutor, InvocationContext, TaskContract};

/// 返回 (work 节点列表, 是否采用模型计划)。
pub async fn plan_work_nodes(
    executor: &dyn AgentExecutor,
    ctx: &InvocationContext,
    request: &str,
    criteria: &[AcceptanceCriterion],
) -> (Vec<TaskNode>, bool) {
    plan_work_nodes_with_roster(executor, ctx, request, criteria, "").await
}

/// 附带候选智能体花名册的计划生成。
pub async fn plan_work_nodes_with_roster(
    executor: &dyn AgentExecutor,
    ctx: &InvocationContext,
    request: &str,
    criteria: &[AcceptanceCriterion],
    roster_context: &str,
) -> (Vec<TaskNode>, bool) {
    let single = vec![work_node("work-1", request, criteria, vec![])];
    let context_pack = if roster_context.trim().is_empty() {
        format!("冻结需求：\n{request}")
    } else {
        format!("冻结需求：\n{request}\n\n{roster_context}")
    };
    let contract = TaskContract {
        objective: format!(
            "把任务拆为 2–4 个可独立执行的 work 子任务 DAG。\n任务：{request}\n\
             每个节点：node_id(work-1..work-4)/assigned_agent_id(可选，从花名册选择)/objective/dependencies(仅引用更早节点)/\
             criterion_ids/read_scopes/write_scopes/resource_locks。\n\
             必需验收项必须至少归属一个节点；写范围不重叠的节点才能并行。"
        ),
        context_pack,
        criteria: criteria.to_vec(),
        inputs: vec![],
    };
    let Ok(result) = executor.execute(ctx, &contract).await else {
        return (single, false);
    };
    let Some(value) = result.structured.or_else(|| extract_json_envelope(&result.text)) else {
        return (single, false);
    };
    let Some(nodes_json) = value["nodes"].as_array() else {
        return (single, false);
    };
    let mut nodes = Vec::new();
    for item in nodes_json {
        let Ok(node) = serde_json::from_value::<TaskNode>(item.clone()) else {
            return (single, false);
        };
        nodes.push(node);
    }
    if validate_dag(&nodes, criteria) {
        (nodes, true)
    } else {
        (single, false)
    }
}

pub fn work_node(
    node_id: &str,
    objective: &str,
    criteria: &[AcceptanceCriterion],
    dependencies: Vec<String>,
) -> TaskNode {
    TaskNode {
        node_id: node_id.to_string(),
        kind: NodeKind::Work,
        dependencies,
        assigned_agent_id: None,
        assigned_agent_name: None,
        assigned_agent_avatar: None,
        assigned_backend: None,
        objective: objective.to_string(),
        input_refs: vec![],
        output_contract: "worker envelope".into(),
        criterion_ids: criteria.iter().map(|c| c.id.clone()).collect(),
        read_scopes: vec![],
        write_scopes: vec![],
        resource_locks: vec![],
        permission_profile_id: String::new(),
        max_attempts: 2,
    }
}

/// 无环、依赖存在、必需项有归属、id 合法且唯一。
pub fn validate_dag(nodes: &[TaskNode], criteria: &[AcceptanceCriterion]) -> bool {
    if nodes.len() < 2 || nodes.len() > 4 {
        return false;
    }
    let ids: Vec<&str> = nodes.iter().map(|n| n.node_id.as_str()).collect();
    for (index, node) in nodes.iter().enumerate() {
        if node.kind != NodeKind::Work || !ids.contains(&node.node_id.as_str()) {
            return false;
        }
        if ids[..index].iter().filter(|id| **id == node.node_id).count() > 0 {
            return false;
        }
        for dep in &node.dependencies {
            if !ids[..index].contains(&dep.as_str()) {
                return false;
            }
        }
    }
    for criterion in criteria.iter().filter(|c| c.required) {
        if !nodes.iter().any(|n| n.criterion_ids.contains(&criterion.id)) {
            return false;
        }
    }
    true
}

/// 读写冲突：写范围路径重叠或资源锁相同。没有 DAG 依赖不等于没有资源冲突。
pub fn conflicts(a: &TaskNode, b: &TaskNode) -> bool {
    if a.resource_locks.iter().any(|lock| b.resource_locks.contains(lock)) {
        return true;
    }
    a.write_scopes.iter().any(|x| {
        b.write_scopes
            .iter()
            .any(|y| x == y || x.starts_with(&format!("{y}/")) || y.starts_with(&format!("{x}/")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::contracts::{base_criteria, TaskType};

    fn criteria() -> Vec<AcceptanceCriterion> {
        base_criteria(TaskType::Coding)
    }

    #[test]
    fn dag_rejects_cycles_missing_deps_and_uncovered_criteria() {
        let c = criteria();
        let mut a = work_node("work-1", "a", &c, vec![]);
        let mut b = work_node("work-2", "b", &c, vec!["work-1".into()]);
        assert!(validate_dag(&[a.clone(), b.clone()], &c));
        b.dependencies = vec!["work-2".into()];
        assert!(!validate_dag(&[a.clone(), b.clone()], &c), "自环");
        b.dependencies = vec!["work-9".into()];
        assert!(!validate_dag(&[a.clone(), b.clone()], &c), "依赖不存在");
        b.dependencies = vec![];
        b.criterion_ids = vec![];
        a.criterion_ids = vec![];
        assert!(!validate_dag(&[a.clone(), b], &c), "必需项无归属");
        a.kind = NodeKind::Review;
        assert!(!validate_dag(&[a], &c), "单节点不算 DAG");
    }

    #[test]
    fn write_scope_overlap_and_locks_conflict() {
        let c = criteria();
        let mut a = work_node("work-1", "a", &c, vec![]);
        let mut b = work_node("work-2", "b", &c, vec![]);
        a.write_scopes = vec!["src/module".into()];
        b.write_scopes = vec!["src/module/sub".into()];
        assert!(conflicts(&a, &b), "路径重叠");
        b.write_scopes = vec!["src/other".into()];
        assert!(!conflicts(&a, &b));
        b.resource_locks = vec!["browser-page-1".into()];
        a.resource_locks = vec!["browser-page-1".into()];
        assert!(conflicts(&a, &b), "同资源锁");
    }
}