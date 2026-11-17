//! 确定性验证器（第 9.1 节）：结构/命令/来源核对由代码判定；
//! 语义审查交给模型审核。执行者自述“全部测试通过”不是工具证据。

use super::contracts::{AcceptanceCriterion, CheckStatus, EvidenceRecord, VerificationKind};

#[derive(Debug, Clone)]
pub struct ValidationCheck {
    pub criterion_id: String,
    pub status: CheckStatus,
    pub explanation: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ValidationOutcome {
    pub checks: Vec<ValidationCheck>,
    /// 环境/权限/依赖缺失：进入受阻，不算执行者质量不合格。
    pub environment_blocked: bool,
}

impl ValidationOutcome {
    pub fn required_failures(&self, criteria: &[AcceptanceCriterion]) -> Vec<String> {
        criteria
            .iter()
            .filter(|c| c.required)
            .filter_map(|c| {
                self.checks
                    .iter()
                    .find(|check| check.criterion_id == c.id)
                    .filter(|check| check.status == CheckStatus::Fail)
                    .map(|_| c.id.clone())
            })
            .collect()
    }

    pub fn required_unverified(&self, criteria: &[AcceptanceCriterion]) -> Vec<String> {
        criteria
            .iter()
            .filter(|c| c.required)
            .filter_map(|c| {
                self.checks
                    .iter()
                    .find(|check| check.criterion_id == c.id)
                    .filter(|check| check.status == CheckStatus::Unverified)
                    .map(|_| c.id.clone())
            })
            .collect()
    }
}

pub fn validate(
    criteria: &[AcceptanceCriterion],
    evidence: &[EvidenceRecord],
    artifact_items: &[(String, String)],
) -> ValidationOutcome {
    let mut outcome = ValidationOutcome::default();
    for criterion in criteria {
        let (status, explanation, refs) = match criterion.verification {
            VerificationKind::Structural => {
                if artifact_items.is_empty() {
                    (CheckStatus::Fail, "没有可核对的交付物".into(), vec![])
                } else if artifact_items.iter().any(|(_, content)| content.trim().is_empty()) {
                    (CheckStatus::Fail, "交付物为空".into(), vec![])
                } else {
                    (CheckStatus::Pass, "交付物存在且非空，结构检查通过".into(), vec![])
                }
            }
            VerificationKind::Command => {
                let commands: Vec<&EvidenceRecord> =
                    evidence.iter().filter(|e| e.kind == "command").collect();
                if commands.is_empty() {
                    (
                        CheckStatus::Unverified,
                        "缺少真实命令证据（退出码/日志），自述通过不算".into(),
                        vec![],
                    )
                } else if commands.iter().any(|e| e.exit_code == Some(0)) {
                    (
                        CheckStatus::Pass,
                        "存在退出码 0 的命令证据".into(),
                        commands.iter().map(|e| e.evidence_id.clone()).collect(),
                    )
                } else {
                    (
                        CheckStatus::Fail,
                        "命令证据退出码非 0".into(),
                        commands.iter().map(|e| e.evidence_id.clone()).collect(),
                    )
                }
            }
            VerificationKind::SourceCheck => {
                let sources: Vec<&EvidenceRecord> = evidence
                    .iter()
                    .filter(|e| e.kind == "source" || (e.kind == "tool" && !e.log_ref.is_empty()))
                    .collect();
                if sources.is_empty() {
                    (CheckStatus::Unverified, "缺少来源/读取证据，语义核对交由审核".into(), vec![])
                } else {
                    (
                        CheckStatus::Pass,
                        "存在来源证据；事实语义由审核者核对".into(),
                        sources.iter().map(|e| e.evidence_id.clone()).collect(),
                    )
                }
            }
            VerificationKind::ModelReview | VerificationKind::User => (
                CheckStatus::Unverified,
                "需模型审核或用户确认".into(),
                vec![],
            ),
        };
        outcome.checks.push(ValidationCheck {
            criterion_id: criterion.id.clone(),
            status,
            explanation,
            evidence_refs: refs,
        });
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::contracts::{base_criteria, TaskType};
    use super::super::persistence::{new_id, unix_now};

    fn evidence(kind: &str, exit: Option<i32>) -> EvidenceRecord {
        EvidenceRecord {
            evidence_id: new_id("ev"),
            source_invocation_id: "inv".into(),
            kind: kind.into(),
            summary: "cargo test".into(),
            cwd: String::new(),
            exit_code: exit,
            artifact_refs: vec![],
            log_ref: "log".into(),
            created_at: unix_now(),
        }
    }

    #[test]
    fn forged_pass_text_without_command_evidence_is_unverified() {
        let criteria = base_criteria(TaskType::Coding);
        let outcome = validate(&criteria, &[], &[("reply".into(), "全部测试通过 PASS".into())]);
        assert_eq!(outcome.checks[0].status, CheckStatus::Unverified);
        assert!(!outcome.required_failures(&criteria).is_empty() || !outcome.required_unverified(&criteria).is_empty());
    }

    #[test]
    fn command_evidence_with_zero_exit_passes_nonzero_fails() {
        let criteria = base_criteria(TaskType::Coding);
        let ok = validate(&criteria, &[evidence("command", Some(0))], &[("r".into(), "x".into())]);
        assert_eq!(ok.checks[0].status, CheckStatus::Pass);
        let bad = validate(&criteria, &[evidence("command", Some(1))], &[("r".into(), "x".into())]);
        assert_eq!(bad.checks[0].status, CheckStatus::Fail);
        assert_eq!(bad.required_failures(&criteria), vec!["checks-pass".to_string()]);
    }

    #[test]
    fn structural_empty_artifact_fails() {
        let criteria = base_criteria(TaskType::Document);
        let outcome = validate(&criteria, &[], &[("doc".into(), "  ".into())]);
        assert_eq!(outcome.checks[0].status, CheckStatus::Fail);
    }
}