//! 独立任务评估（第 5 节）：必经评估调用、输出校验、最多一次受限格式修复。
//! 评估无效仍无效 ⇒ 受阻，不能默认当作简单任务。

use super::contracts::{base_criteria, AcceptanceCriterion, InvocationResult, TaskAssessment};
use super::executor::{
    extract_json_envelope, truncate_utf8, AgentExecutor, ExecError, InvocationContext,
    TaskContract,
};
use super::persistence::new_id;

#[derive(Debug)]
pub enum TriageError {
    Blocked(String),
    Transient(String),
    Aborted(String),
}

impl From<ExecError> for TriageError {
    fn from(error: ExecError) -> Self {
        if error.aborted {
            Self::Aborted(error.message)
        } else if error.transient {
            Self::Transient(error.message)
        } else {
            Self::Blocked(error.message)
        }
    }
}

#[derive(Debug)]
pub struct AssessOutcome {
    pub assessment: TaskAssessment,
    /// 含修复调用；计入调用统计。
    pub invocations_used: u32,
    pub repaired: bool,
    pub result: InvocationResult,
    pub repair_result: Option<(String, u32, InvocationResult)>,
}

pub async fn assess(
    executor: &dyn AgentExecutor,
    ctx: &InvocationContext,
    request: &str,
    task_revision: u32,
    repair_attempt: u32,
) -> Result<AssessOutcome, TriageError> {
    assess_with_roster(executor, ctx, request, "", task_revision, repair_attempt).await
}

pub async fn assess_with_roster(
    executor: &dyn AgentExecutor,
    ctx: &InvocationContext,
    request: &str,
    roster_context: &str,
    task_revision: u32,
    repair_attempt: u32,
) -> Result<AssessOutcome, TriageError> {
    let context_pack = if roster_context.trim().is_empty() {
        format!("冻结的用户原始要求（需求版本 {task_revision}）：\n{request}")
    } else {
        format!("冻结的用户原始要求（需求版本 {task_revision}）：\n{request}\n\n{roster_context}")
    };
    let contract = TaskContract {
        objective: request.to_string(),
        context_pack,
        ..Default::default()
    };
    let first = executor.execute(ctx, &contract).await?;
    let mut invocations_used = 1;
    let mut repaired = false;
    let mut repair_result = None;
    let mut primary_result = first.clone();

    let parsed = parse_and_validate(&first, task_revision);
    let assessment = match parsed {
        Ok(assessment) => assessment,
        Err(first_error) => {
            // 受限格式修复：只修 JSON，不重新评估；仍无效则受阻。
            let mut repair_ctx = ctx.clone();
            repair_ctx.invocation_id = new_id("inv");
            repair_ctx.attempt = repair_attempt;
            let repair_contract = TaskContract {
                objective: format!(
                    "上一次评估输出未通过校验：{first_error}\n原始输出：\n{}\n只输出修正后的完整 JSON（字段不变），不要重新评估，不要附加解释。",
                    truncate_utf8(&first.text, 4000)
                ),
                ..Default::default()
            };
            let second = executor.execute(&repair_ctx, &repair_contract).await?;
            invocations_used += 1;
            repaired = true;
            let assessment = parse_and_validate(&second, task_revision)
                .map_err(|e| TriageError::Blocked(format!("评估 JSON 无效且修复失败：{e}")))?;
            repair_result = Some((repair_ctx.invocation_id, repair_attempt, second.clone()));
            primary_result = second;
            assessment
        }
    };
    Ok(AssessOutcome {
        assessment,
        invocations_used,
        repaired,
        result: primary_result,
        repair_result,
    })
}

fn parse_and_validate(
    result: &super::contracts::InvocationResult,
    task_revision: u32,
) -> Result<TaskAssessment, String> {
    let value = result
        .structured
        .clone()
        .or_else(|| extract_json_envelope(&result.text))
        .ok_or_else(|| "缺少 JSON 封套".to_string())?;
    let mut assessment: TaskAssessment =
        serde_json::from_value(value).map_err(|e| format!("字段不合法：{e}"))?;
    // 身份/版本由可信运行时封装，模型自报无效。
    assessment.task_revision = task_revision;
    assessment.validate().map_err(|e| e.to_string())?;
    Ok(assessment)
}

/// 验收合同冻结：类型标准项 + 评估补充项；必需项不得为空（第 9.1 节）。
pub fn freeze_criteria(assessment: &TaskAssessment) -> Vec<AcceptanceCriterion> {
    let mut criteria = base_criteria(assessment.task_type);
    for extra in &assessment.acceptance_criteria {
        if !criteria.iter().any(|c| c.id == extra.id) {
            criteria.push(extra.clone());
        }
    }
    if !criteria.iter().any(|c| c.required) {
        if let Some(first) = criteria.first_mut() {
            first.required = true;
        }
    }
    criteria
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::executor::FakeExecutor;
    use super::super::contracts::{InvocationResult, Usage};
    use std::path::PathBuf;
    use tokio_util::sync::CancellationToken;

    fn ctx() -> InvocationContext {
        InvocationContext {
            session_id: "s".into(),
            task_id: "t".into(),
            run_id: "r".into(),
            node_id: "assess".into(),
            invocation_id: "inv-assess".into(),
            attempt: 1,
            role: super::super::contracts::NodeKind::Assess,
            backend: "fake".into(),
            model: None,
            effort: None,
            workspace: PathBuf::from("."),
            permission: String::new(),
            read_only: true,
            timeout_secs: 60,
            cancel: CancellationToken::new(),
            entry: None,
            policy_version: 1,
            prompt_version: 1,
            agent: None,
        }
    }

    fn assessment_json() -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1,
            "task_revision": 99,
            "task_type": "writing",
            "tags": [],
            "scores": {"scope":0,"uncertainty":0,"reasoning":0,"dependencies":0,"verification":0,"effort":0},
            "suggested_difficulty": "L0",
            "suggested_risk": "R0",
            "confidence": 0.9,
            "rationale": ["单一改写"],
            "missing_information": [],
            "required_capabilities": [],
            "acceptance_criteria": [],
            "parallelizable": false
        })
    }

    fn result_with(structured: Option<serde_json::Value>, text: &str) -> InvocationResult {
        InvocationResult {
            invocation_id: String::new(),
            node_id: String::new(),
            attempt: 1,
            text: text.to_string(),
            structured,
            usage: Usage::default(),
            evidence: vec![],
            error: None,
            aborted: false,
        }
    }

    #[test]
    fn valid_assessment_passes_and_runtime_overrides_revision() {
        let executor = FakeExecutor::new();
        executor.push(None, Ok(result_with(Some(assessment_json()), "```json\n{}\n```")));
        let outcome = futures_util::FutureExt::now_or_never(assess(&executor, &ctx(), "改写这段话", 3, 2))
            .unwrap()
            .unwrap();
        assert_eq!(outcome.invocations_used, 1);
        assert!(!outcome.repaired);
        assert_eq!(outcome.assessment.task_revision, 3, "模型自报版本必须被运行时覆盖");
    }

    #[test]
    fn invalid_json_gets_one_limited_repair_then_succeeds() {
        let executor = FakeExecutor::new();
        executor.push(None, Ok(result_with(None, "我觉得很简单 PASS")));
        executor.push(None, Ok(result_with(Some(assessment_json()), "fixed")));
        let outcome = futures_util::FutureExt::now_or_never(assess(&executor, &ctx(), "t", 1, 2))
            .unwrap()
            .unwrap();
        assert_eq!(outcome.invocations_used, 2);
        assert!(outcome.repaired);
    }

    #[test]
    fn twice_invalid_blocks_never_defaults_to_simple() {
        let executor = FakeExecutor::new();
        executor.push(None, Ok(result_with(None, "garbage")));
        executor.push(None, Ok(result_with(None, "still garbage")));
        let error = futures_util::FutureExt::now_or_never(assess(&executor, &ctx(), "t", 1, 2))
            .unwrap()
            .unwrap_err();
        assert!(matches!(error, TriageError::Blocked(_)));
    }

    #[test]
    fn freeze_criteria_keeps_type_baseline_and_adds_extras() {
        let mut assessment: TaskAssessment = serde_json::from_value(assessment_json()).unwrap();
        assessment.acceptance_criteria.push(AcceptanceCriterion {
            id: "tone".into(),
            requirement: "语气正式".into(),
            required: false,
            verification: super::super::contracts::VerificationKind::ModelReview,
            evidence_requirements: vec![],
        });
        let criteria = freeze_criteria(&assessment);
        assert!(criteria.iter().any(|c| c.id == "cover-request" && c.required));
        assert!(criteria.iter().any(|c| c.id == "tone"));
    }

    #[test]
    fn transient_exec_error_propagates_as_transient() {
        let executor = FakeExecutor::new();
        executor.push(None, Err(ExecError::transient("network")));
        let error = futures_util::FutureExt::now_or_never(assess(&executor, &ctx(), "t", 1, 2))
            .unwrap()
            .unwrap_err();
        assert!(matches!(error, TriageError::Transient(_)));
    }
}