//! 智能调度核心数据合同（docs/adaptive-task-orchestration-plan.md 第 8 节）。
//! 模型只填写内容字段；身份、版本、时间等由可信运行时封装。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const ASSESSMENT_SCHEMA_VERSION: u8 = 1;
pub const REVIEW_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Difficulty {
    L0,
    L1,
    L2,
    L3,
}

impl Difficulty {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::L0 => "L0",
            Self::L1 => "L1",
            Self::L2 => "L2",
            Self::L3 => "L3",
        }
    }

    /// 并发上限初值；首版串行 MVP 在 policy 里再压到 1。
    pub fn concurrency_cap(self) -> u32 {
        match self {
            Self::L0 | Self::L1 => 1,
            Self::L2 => 2,
            Self::L3 => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Risk {
    R0,
    R1,
    R2,
    R3,
}

impl Risk {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::R0 => "R0",
            Self::R1 => "R1",
            Self::R2 => "R2",
            Self::R3 => "R3",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    Qa,
    Writing,
    Research,
    Coding,
    DataAnalysis,
    Document,
    ComputerOperation,
    Mixed,
}

impl TaskType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Qa => "qa",
            Self::Writing => "writing",
            Self::Research => "research",
            Self::Coding => "coding",
            Self::DataAnalysis => "data_analysis",
            Self::Document => "document",
            Self::ComputerOperation => "computer_operation",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationKind {
    Structural,
    Command,
    SourceCheck,
    ModelReview,
    User,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    pub id: String,
    pub requirement: String,
    pub required: bool,
    pub verification: VerificationKind,
    #[serde(default)]
    pub evidence_requirements: Vec<String>,
}

/// 六维评分，每维 0–2。用结构体而不是 map，非法维度名在反序列化时即失败。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Scores {
    pub scope: u8,
    pub uncertainty: u8,
    pub reasoning: u8,
    pub dependencies: u8,
    pub verification: u8,
    pub effort: u8,
}

impl Scores {
    pub fn total(&self) -> u8 {
        self.scope + self.uncertainty + self.reasoning + self.dependencies + self.verification + self.effort
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        for (name, value) in [
            ("scope", self.scope),
            ("uncertainty", self.uncertainty),
            ("reasoning", self.reasoning),
            ("dependencies", self.dependencies),
            ("verification", self.verification),
            ("effort", self.effort),
        ] {
            if value > 2 {
                return Err(ContractError::Invalid(format!("scores.{name} 超出 0–2：{value}")));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAssessment {
    pub schema_version: u8,
    #[serde(default)]
    pub task_revision: u32,
    pub task_type: TaskType,
    #[serde(default)]
    pub tags: Vec<String>,
    pub scores: Scores,
    pub suggested_difficulty: Difficulty,
    pub suggested_risk: Risk,
    pub confidence: f32,
    #[serde(default)]
    pub rationale: Vec<String>,
    #[serde(default)]
    pub missing_information: Vec<String>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    #[serde(default)]
    pub parallelizable: bool,
    #[serde(default)]
    pub recommended_agent_id: Option<String>,
    #[serde(default)]
    pub recommended_reason: Option<String>,
}

impl TaskAssessment {
    /// 模型输出是待校验数据：schema、范围、枚举、引用完整性都在这里把关。
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != ASSESSMENT_SCHEMA_VERSION {
            return Err(ContractError::Invalid(format!(
                "assessment schema_version 必须为 {ASSESSMENT_SCHEMA_VERSION}，收到 {}",
                self.schema_version
            )));
        }
        self.scores.validate()?;
        if !(0.0..=1.0).contains(&self.confidence) || self.confidence.is_nan() {
            return Err(ContractError::Invalid(format!("confidence 超出 0–1：{}", self.confidence)));
        }
        if self.rationale.len() > 12 || self.missing_information.len() > 12 {
            return Err(ContractError::Invalid("rationale/missing_information 过长".into()));
        }
        let mut ids = std::collections::HashSet::new();
        for criterion in &self.acceptance_criteria {
            if criterion.id.trim().is_empty() || !ids.insert(criterion.id.clone()) {
                return Err(ContractError::Invalid(format!("验收项 id 空或重复：{}", criterion.id)));
            }
            if criterion.requirement.trim().is_empty() || criterion.requirement.len() > 2000 {
                return Err(ContractError::Invalid(format!("验收项 {} requirement 空或过长", criterion.id)));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Assess,
    Plan,
    Work,
    Integrate,
    Validate,
    Review,
    PrepareAction,
    ReviewAction,
    ExecuteAction,
    VerifyResult,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Assess => "assess",
            Self::Plan => "plan",
            Self::Work => "work",
            Self::Integrate => "integrate",
            Self::Validate => "validate",
            Self::Review => "review",
            Self::PrepareAction => "prepare_action",
            Self::ReviewAction => "review_action",
            Self::ExecuteAction => "execute_action",
            Self::VerifyResult => "verify_result",
        }
    }

    /// 只读职责：执行端能力检查必须拒绝写工具（第 9.2 / 15 节）。
    pub fn read_only(self) -> bool {
        matches!(self, Self::Assess | Self::Review | Self::ReviewAction)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNode {
    pub node_id: String,
    pub kind: NodeKind,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub assigned_agent_id: Option<String>,
    #[serde(default)]
    pub assigned_agent_name: Option<String>,
    #[serde(default)]
    pub assigned_agent_avatar: Option<String>,
    #[serde(default)]
    pub assigned_backend: Option<String>,
    pub objective: String,
    #[serde(default)]
    pub input_refs: Vec<String>,
    #[serde(default)]
    pub output_contract: String,
    #[serde(default)]
    pub criterion_ids: Vec<String>,
    #[serde(default)]
    pub read_scopes: Vec<String>,
    #[serde(default)]
    pub write_scopes: Vec<String>,
    #[serde(default)]
    pub resource_locks: Vec<String>,
    #[serde(default)]
    pub permission_profile_id: String,
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
}

fn default_max_attempts() -> u32 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactItem {
    pub id: String,
    pub uri: String,
    pub media_type: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactManifest {
    pub artifact_set_id: String,
    pub task_revision: u32,
    pub revision: u32,
    pub content_hash: String,
    /// 生成者 invocation ID，由运行时填写，模型不可自报。
    pub produced_by: Vec<String>,
    pub items: Vec<ArtifactItem>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub known_limitations: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Pass,
    Rework,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    Unverified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    Blocker,
    Major,
    Minor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewCheck {
    pub criterion_id: String,
    pub status: CheckStatus,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewIssue {
    pub id: String,
    pub severity: IssueSeverity,
    pub criterion_id: String,
    pub artifact_ref: String,
    pub responsible_node_id: String,
    pub description: String,
    pub required_fix: String,
    pub verification: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewReport {
    pub schema_version: u8,
    pub task_revision: u32,
    pub artifact_set_id: String,
    pub artifact_hash: String,
    pub verdict: ReviewVerdict,
    pub checks: Vec<ReviewCheck>,
    #[serde(default)]
    pub issues: Vec<ReviewIssue>,
}

impl ReviewReport {
    /// 审核输出也是待校验数据。`required_ids` 为冻结合同中的必需验收项。
    /// 结论与缺陷矛盾、必需项缺失、引用无效都拒收（第 9.2 节）。
    pub fn validate(&self, required_ids: &[String], known_nodes: &[String]) -> Result<(), ContractError> {
        if self.schema_version != REVIEW_SCHEMA_VERSION {
            return Err(ContractError::Invalid(format!(
                "review schema_version 必须为 {REVIEW_SCHEMA_VERSION}，收到 {}",
                self.schema_version
            )));
        }
        let mut checked = std::collections::HashSet::new();
        for check in &self.checks {
            if !checked.insert(check.criterion_id.clone()) {
                return Err(ContractError::Invalid(format!("重复验收项检查：{}", check.criterion_id)));
            }
        }
        for required in required_ids {
            let check = self
                .checks
                .iter()
                .find(|c| &c.criterion_id == required)
                .ok_or_else(|| ContractError::Invalid(format!("审核缺少必需验收项：{required}")))?;
            if self.verdict == ReviewVerdict::Pass && check.status != CheckStatus::Pass {
                return Err(ContractError::Invalid(format!(
                    "verdict=pass 但必需项 {} 状态为 {:?}",
                    required, check.status
                )));
            }
        }
        let mut issue_ids = std::collections::HashSet::new();
        for issue in &self.issues {
            if issue.id.trim().is_empty() || !issue_ids.insert(issue.id.clone()) {
                return Err(ContractError::Invalid(format!("缺陷 id 空或重复：{}", issue.id)));
            }
            if !checked.contains(&issue.criterion_id) {
                return Err(ContractError::Invalid(format!("缺陷引用未检查的验收项：{}", issue.criterion_id)));
            }
            if !known_nodes.contains(&issue.responsible_node_id) {
                return Err(ContractError::Invalid(format!(
                    "缺陷责任节点不存在：{}",
                    issue.responsible_node_id
                )));
            }
            if issue.required_fix.trim().is_empty() || issue.verification.trim().is_empty() {
                return Err(ContractError::Invalid(format!("缺陷 {} 缺少可执行修复/复测说明", issue.id)));
            }
        }
        let has_blocking = self
            .issues
            .iter()
            .any(|i| matches!(i.severity, IssueSeverity::Blocker | IssueSeverity::Major));
        match self.verdict {
            ReviewVerdict::Pass if has_blocking => {
                return Err(ContractError::Invalid("verdict=pass 但存在 blocker/major 缺陷".into()));
            }
            ReviewVerdict::Rework if self.issues.is_empty() => {
                return Err(ContractError::Invalid("verdict=rework 必须附带缺陷单".into()));
            }
            _ => {}
        }
        Ok(())
    }

    pub fn blocking_issues(&self) -> Vec<&ReviewIssue> {
        self.issues
            .iter()
            .filter(|i| matches!(i.severity, IssueSeverity::Blocker | IssueSeverity::Major))
            .collect()
    }
}

/// 节点一次真实调用的结果。usage 缺失记 unknown，不伪造精确值（第 6 节）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub model_requests: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub tool_calls: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub evidence_id: String,
    pub source_invocation_id: String,
    /// command / source / file / external
    pub kind: String,
    pub summary: String,
    #[serde(default)]
    pub cwd: String,
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub artifact_refs: Vec<String>,
    #[serde(default)]
    pub log_ref: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvocationResult {
    pub invocation_id: String,
    pub node_id: String,
    pub attempt: u32,
    pub text: String,
    /// 约定封套解析出的结构化输出（assessment / review / subtask 请求等）。
    pub structured: Option<serde_json::Value>,
    pub usage: Usage,
    pub evidence: Vec<EvidenceRecord>,
    pub error: Option<String>,
    pub aborted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Queued,
    Assessing,
    Planning,
    Executing,
    Integrating,
    Validating,
    Reviewing,
    ReworkRequired,
    WaitingInput,
    Blocked,
    Cancelling,
    Cancelled,
    Succeeded,
    Failed,
    ReviewFailed,
    Interrupted,
}

impl RunState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Assessing => "assessing",
            Self::Planning => "planning",
            Self::Executing => "executing",
            Self::Integrating => "integrating",
            Self::Validating => "validating",
            Self::Reviewing => "reviewing",
            Self::ReworkRequired => "rework_required",
            Self::WaitingInput => "waiting_input",
            Self::Blocked => "blocked",
            Self::Cancelling => "cancelling",
            Self::Cancelled => "cancelled",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::ReviewFailed => "review_failed",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "queued" => Self::Queued,
            "assessing" => Self::Assessing,
            "planning" => Self::Planning,
            "executing" => Self::Executing,
            "integrating" => Self::Integrating,
            "validating" => Self::Validating,
            "reviewing" => Self::Reviewing,
            "rework_required" => Self::ReworkRequired,
            "waiting_input" => Self::WaitingInput,
            "blocked" => Self::Blocked,
            "cancelling" => Self::Cancelling,
            "cancelled" => Self::Cancelled,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "review_failed" => Self::ReviewFailed,
            "interrupted" => Self::Interrupted,
            _ => return None,
        })
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::ReviewFailed | Self::Cancelled
        )
    }

    pub fn is_resumable(&self) -> bool {
        matches!(self, Self::Blocked | Self::WaitingInput | Self::Interrupted)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    Pending,
    Running,
    Completed,
    Failed,
    Blocked,
    Cancelled,
    Skipped,
}

impl NodeState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::Cancelled => "cancelled",
            Self::Skipped => "skipped",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "blocked" => Self::Blocked,
            "cancelled" => Self::Cancelled,
            "skipped" => Self::Skipped,
            _ => return None,
        })
    }
}

/// 后端能力矩阵条目（第 10.1 节）。未知能力标 unknown，不臆测。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendCapabilities {
    pub backend: String,
    pub structured_output: Capability,
    pub workspace_override: Capability,
    pub read_only_enforcement: Capability,
    pub cancel: Capability,
    pub usage_reporting: Capability,
    pub resume: Capability,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectivePolicy {
    pub difficulty: Difficulty,
    pub risk: Risk,
    pub force_review: bool,
    /// 高影响外部写：审核前置到执行之前（第 5/7 节）。
    pub pre_review_external_actions: bool,
    pub max_concurrency: u32,
    pub max_rework: u32,
    pub max_transient_retries: u32,
    pub confidence_threshold: f32,
    pub rule_hits: Vec<String>,
    pub policy_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationEvent {
    pub event_id: String,
    pub session_id: String,
    pub task_id: String,
    pub run_id: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub invocation_id: String,
    #[serde(default)]
    pub attempt: u32,
    pub seq: u64,
    pub state_revision: u64,
    #[serde(rename = "type")]
    pub kind: String,
    pub payload: serde_json::Value,
    pub created_at: i64,
}

/// 任务类型 → 标准验收模板（第 9.1 节）。评估者在此基础上补充具体项。
pub fn base_criteria(task_type: TaskType) -> Vec<AcceptanceCriterion> {
    let (id, requirement, verification) = match task_type {
        TaskType::Qa | TaskType::Writing => (
            "cover-request",
            "回复覆盖用户原始要求的全部要点，事实性陈述有来源或明确标注推断",
            VerificationKind::Structural,
        ),
        TaskType::Research => (
            "sources-mapped",
            "结论与来源映射完整，来源含 URL/日期/摘录，事实与推断区分",
            VerificationKind::SourceCheck,
        ),
        TaskType::Coding => (
            "checks-pass",
            "与改动相关的必要检查（编译/测试/复现命令）真实运行且通过，附退出码与日志",
            VerificationKind::Command,
        ),
        TaskType::DataAnalysis => (
            "recompute",
            "输入快照/哈希、计算脚本与口径齐全，复算结果一致",
            VerificationKind::Command,
        ),
        TaskType::Document => (
            "openable",
            "目标文件可打开、页面/表格完整，渲染结果已核对",
            VerificationKind::Structural,
        ),
        TaskType::ComputerOperation => (
            "state-verified",
            "操作前后状态与目标对象记录齐全，实际变更已核对",
            VerificationKind::Structural,
        ),
        TaskType::Mixed => (
            "cover-request",
            "各子目标交付物齐全且分别满足对应类型检查",
            VerificationKind::ModelReview,
        ),
    };
    vec![AcceptanceCriterion {
        id: id.to_string(),
        requirement: requirement.to_string(),
        required: true,
        verification,
        evidence_requirements: vec!["工具运行器保存的命令/来源证据，执行者自述不算证据".to_string()],
    }]
}

/// 供路由比较的候选元数据补充（第 4 节）；无法探测的标 unknown。
pub type CapabilityTags = BTreeMap<String, Capability>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractError {
    Invalid(String),
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ContractError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn assessment() -> TaskAssessment {
        TaskAssessment {
            schema_version: 1,
            task_revision: 1,
            task_type: TaskType::Writing,
            tags: vec![],
            scores: Scores { scope: 0, uncertainty: 0, reasoning: 0, dependencies: 0, verification: 0, effort: 0 },
            suggested_difficulty: Difficulty::L0,
            suggested_risk: Risk::R0,
            confidence: 0.9,
            rationale: vec!["单一改写目标".into()],
            missing_information: vec![],
            required_capabilities: vec![],
            acceptance_criteria: base_criteria(TaskType::Writing),
            parallelizable: false,
            recommended_agent_id: None,
            recommended_reason: None,
        }
    }

    #[test]
    fn assessment_rejects_bad_schema_scores_confidence_and_dup_ids() {
        let mut a = assessment();
        assert!(a.validate().is_ok());
        a.schema_version = 2;
        assert!(a.validate().is_err());
        a.schema_version = 1;
        a.scores.scope = 3;
        assert!(a.validate().is_err());
        a.scores.scope = 0;
        a.confidence = 1.5;
        assert!(a.validate().is_err());
        a.confidence = 0.9;
        a.acceptance_criteria.push(a.acceptance_criteria[0].clone());
        assert!(a.validate().is_err());
    }

    #[test]
    fn review_pass_requires_all_required_checks_and_no_blocking_issues() {
        let required = vec!["cover-request".to_string()];
        let nodes = vec!["work-1".to_string()];
        let mut report = ReviewReport {
            schema_version: 1,
            task_revision: 1,
            artifact_set_id: "set-1".into(),
            artifact_hash: "hash".into(),
            verdict: ReviewVerdict::Pass,
            checks: vec![ReviewCheck {
                criterion_id: "cover-request".into(),
                status: CheckStatus::Pass,
                evidence_refs: vec!["ev-1".into()],
                explanation: "覆盖".into(),
            }],
            issues: vec![],
        };
        assert!(report.validate(&required, &nodes).is_ok());
        // 缺必需项检查
        report.checks.clear();
        assert!(report.validate(&required, &nodes).is_err());
        // 必需项 unverified 却 pass
        report.checks.push(ReviewCheck {
            criterion_id: "cover-request".into(),
            status: CheckStatus::Unverified,
            evidence_refs: vec![],
            explanation: String::new(),
        });
        assert!(report.validate(&required, &nodes).is_err());
        // pass 但带 major 缺陷
        report.checks[0].status = CheckStatus::Pass;
        report.issues.push(ReviewIssue {
            id: "i1".into(),
            severity: IssueSeverity::Major,
            criterion_id: "cover-request".into(),
            artifact_ref: "art-1".into(),
            responsible_node_id: "work-1".into(),
            description: "漏要点".into(),
            required_fix: "补要点".into(),
            verification: "核对原文".into(),
        });
        assert!(report.validate(&required, &nodes).is_err());
        // rework 无缺陷单
        report.issues.clear();
        report.verdict = ReviewVerdict::Rework;
        assert!(report.validate(&required, &nodes).is_err());
    }

    #[test]
    fn run_state_terminal_and_resumable_sets_match_plan() {
        for state in [RunState::Succeeded, RunState::Failed, RunState::ReviewFailed, RunState::Cancelled] {
            assert!(state.is_terminal(), "{}", state.as_str());
            assert!(!state.is_resumable());
        }
        for state in [RunState::Blocked, RunState::WaitingInput, RunState::Interrupted] {
            assert!(state.is_resumable());
            assert!(!state.is_terminal());
        }
        assert_eq!(RunState::parse("rework_required"), Some(RunState::ReworkRequired));
        assert_eq!(RunState::parse("nope"), None);
    }

    #[test]
    fn read_only_roles_are_assess_and_review() {
        assert!(NodeKind::Assess.read_only());
        assert!(NodeKind::Review.read_only());
        assert!(NodeKind::ReviewAction.read_only());
        assert!(!NodeKind::Work.read_only());
        assert!(!NodeKind::ExecuteAction.read_only());
    }
}