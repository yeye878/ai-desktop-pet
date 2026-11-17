//! 确定性策略约束（第 5/6 节）：模型提建议，程序执行约束。
//! 后续流程只读 EffectivePolicy，不直接执行模型返回的任意参数。

use crate::orchestration::contracts::{
    AcceptanceCriterion, Difficulty, EffectivePolicy, Risk, TaskAssessment, TaskType,
    VerificationKind,
};

pub const POLICY_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct PolicyConfig {
    /// 串行 MVP 开关；P3 起默认开放并行（仍受等级上限与配置约束）。
    pub serial_mvp: bool,
    pub confidence_threshold: f32,
    pub max_rework: u32,
    pub max_transient_retries: u32,
    pub max_concurrency: u32,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            serial_mvp: false,
            confidence_threshold: 0.75,
            max_rework: 2,
            max_transient_retries: 1,
            max_concurrency: 3,
        }
    }
}

pub fn score_difficulty(total: u8) -> Difficulty {
    match total {
        0..=2 => Difficulty::L0,
        3..=5 => Difficulty::L1,
        6..=8 => Difficulty::L2,
        _ => Difficulty::L3,
    }
}

/// 评估建议 → 有效策略。只升不降：模型不能自行取消审核或放宽门槛。
pub fn compute_effective_policy(
    assessment: &TaskAssessment,
    has_external_writes: bool,
    config: &PolicyConfig,
) -> EffectivePolicy {
    let mut rule_hits = Vec::new();
    let scored = score_difficulty(assessment.scores.total());
    let mut difficulty = assessment.suggested_difficulty.max(scored);
    if difficulty != assessment.suggested_difficulty {
        rule_hits.push(format!(
            "difficulty_floor: scores={} -> {} (建议 {})",
            assessment.scores.total(),
            difficulty.as_str(),
            assessment.suggested_difficulty.as_str()
        ));
    }
    if assessment.confidence < config.confidence_threshold && difficulty == Difficulty::L0 {
        difficulty = Difficulty::L1;
        rule_hits.push(format!(
            "confidence_floor: {} < {} -> L1",
            assessment.confidence, config.confidence_threshold
        ));
    }
    let risk = assessment.suggested_risk;

    let force_review = difficulty >= Difficulty::L2 || risk >= Risk::R2;
    if force_review {
        rule_hits.push(format!(
            "mandatory_review: difficulty={} risk={}",
            difficulty.as_str(),
            risk.as_str()
        ));
    }
    let pre_review_external_actions = has_external_writes && risk >= Risk::R2;
    if pre_review_external_actions {
        rule_hits.push("pre_review_external_actions: risk>=R2 且含外部写".into());
    }

    let cap = if config.serial_mvp {
        1
    } else {
        difficulty.concurrency_cap().min(config.max_concurrency)
    };

    EffectivePolicy {
        difficulty,
        risk,
        force_review,
        pre_review_external_actions,
        max_concurrency: cap,
        max_rework: config.max_rework,
        max_transient_retries: config.max_transient_retries,
        confidence_threshold: config.confidence_threshold,
        rule_hits,
        policy_version: POLICY_VERSION,
    }
}

/// 若冻结验收合同包含必需的 model_review 项，强制开启独立审核，避免无审核者却在验证期死锁。
pub fn enforce_criteria_review(policy: &mut EffectivePolicy, criteria: &[AcceptanceCriterion]) {
    if !policy.force_review
        && criteria
            .iter()
            .any(|c| c.required && c.verification == VerificationKind::ModelReview)
    {
        policy.force_review = true;
        policy
            .rule_hits
            .push("criteria_review_floor: required model_review -> force_review".into());
    }
}

/// 高影响外部写的确定性识别：能力标签或任务类型命中，不只靠关键词。
pub fn has_external_writes(assessment: &TaskAssessment) -> bool {
    assessment
        .required_capabilities
        .iter()
        .any(|cap| cap == "external_write" || cap == "publish" || cap == "send_message" || cap == "delete")
        || assessment.task_type == TaskType::ComputerOperation
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration::contracts::{base_criteria, Scores, TaskType};

    fn assessment(scores: Scores, suggested: Difficulty, risk: Risk, confidence: f32) -> TaskAssessment {
        TaskAssessment {
            schema_version: 1,
            task_revision: 1,
            task_type: TaskType::Coding,
            tags: vec![],
            scores,
            suggested_difficulty: suggested,
            suggested_risk: risk,
            confidence,
            rationale: vec![],
            missing_information: vec![],
            required_capabilities: vec![],
            acceptance_criteria: base_criteria(TaskType::Coding),
            parallelizable: false,
            recommended_agent_id: None,
            recommended_reason: None,
        }
    }

    fn scores(total_each: u8) -> Scores {
        Scores {
            scope: total_each,
            uncertainty: total_each,
            reasoning: total_each,
            dependencies: total_each,
            verification: total_each,
            effort: total_each,
        }
    }

    #[test]
    fn score_boundaries_match_plan() {
        assert_eq!(score_difficulty(0), Difficulty::L0);
        assert_eq!(score_difficulty(2), Difficulty::L0);
        assert_eq!(score_difficulty(3), Difficulty::L1);
        assert_eq!(score_difficulty(5), Difficulty::L1);
        assert_eq!(score_difficulty(6), Difficulty::L2);
        assert_eq!(score_difficulty(8), Difficulty::L2);
        assert_eq!(score_difficulty(9), Difficulty::L3);
        assert_eq!(score_difficulty(12), Difficulty::L3);
    }

    #[test]
    fn model_cannot_downgrade_below_scores() {
        // 每维 2 => 12 分 L3；模型建议 L0 无效。
        let a = assessment(scores(2), Difficulty::L0, Risk::R0, 0.95);
        let p = compute_effective_policy(&a, false, &PolicyConfig::default());
        assert_eq!(p.difficulty, Difficulty::L3);
        assert!(p.force_review);
        assert!(p.rule_hits.iter().any(|h| h.starts_with("difficulty_floor")));
    }

    #[test]
    fn low_confidence_cannot_take_l0_fast_path() {
        let a = assessment(scores(0), Difficulty::L0, Risk::R0, 0.5);
        let p = compute_effective_policy(&a, false, &PolicyConfig::default());
        assert_eq!(p.difficulty, Difficulty::L1);
        assert!(p.rule_hits.iter().any(|h| h.starts_with("confidence_floor")));
    }

    #[test]
    fn short_but_high_risk_still_requires_review() {
        // 难度 L0 但 R2：必须独立审核，不能走快速通道。
        let a = assessment(scores(0), Difficulty::L0, Risk::R2, 0.95);
        let p = compute_effective_policy(&a, true, &PolicyConfig::default());
        assert_eq!(p.difficulty, Difficulty::L0);
        assert!(p.force_review);
        assert!(p.pre_review_external_actions);
    }

    #[test]
    fn l0_r0_fast_path_has_no_forced_review() {
        let a = assessment(scores(0), Difficulty::L0, Risk::R0, 0.9);
        let p = compute_effective_policy(&a, false, &PolicyConfig::default());
        assert!(!p.force_review);
        assert!(!p.pre_review_external_actions);
        assert_eq!(p.max_rework, 2);
        assert_eq!(p.max_transient_retries, 1);
    }

    #[test]
    fn serial_mvp_caps_concurrency_at_one_for_all_levels() {
        let a = assessment(scores(2), Difficulty::L3, Risk::R1, 0.9);
        let mut config = PolicyConfig::default();
        config.serial_mvp = true;
        let p = compute_effective_policy(&a, false, &config);
        assert_eq!(p.max_concurrency, 1);
        let p = compute_effective_policy(&a, false, &PolicyConfig::default());
        assert_eq!(p.max_concurrency, 3);
    }

    #[test]
    fn external_write_detection_uses_capabilities_and_type() {
        let mut a = assessment(scores(0), Difficulty::L0, Risk::R1, 0.9);
        assert!(!has_external_writes(&a));
        a.required_capabilities.push("external_write".into());
        assert!(has_external_writes(&a));
        a.required_capabilities.clear();
        a.task_type = TaskType::ComputerOperation;
        assert!(has_external_writes(&a));
        // R1 外部写不触发前置审核链，但仍受已有授权约束。
        let p = compute_effective_policy(&a, true, &PolicyConfig::default());
        assert!(!p.pre_review_external_actions);
    }
}