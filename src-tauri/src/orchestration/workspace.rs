//! 工作区隔离（第 10.3 节）：Git 代码写入优先各自 worktree，基线包含未提交改动；
//! 隔离失败降为串行。非 Git 任务使用独立输出目录。

use std::path::{Path, PathBuf};
use std::process::Command;

fn git(base: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(base).args(args);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let output = cmd.output().map_err(|e| format!("git 不可用：{e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {:?} 失败：{}",
            args,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn is_git_repo(base: &Path) -> bool {
    git(base, &["rev-parse", "--is-inside-work-tree"])
        .map(|out| out.trim() == "true")
        .unwrap_or(false)
}

/// 为写节点建立隔离 worktree：HEAD 基线 + 未提交 tracked 差异 + untracked 文件。
/// 任一步失败返回 Err，调度器据此降为串行，不静默覆盖用户工作区。
pub fn prepare_isolated(base: &Path, run_dir: &Path, node_id: &str) -> Result<PathBuf, String> {
    if !is_git_repo(base) {
        return Err("非 Git 仓库".into());
    }
    let wt = run_dir.join("worktrees").join(node_id);
    if wt.exists() {
        let _ = git(base, &["worktree", "remove", "--force", &wt.to_string_lossy()]);
    }
    std::fs::create_dir_all(wt.parent().unwrap_or(run_dir)).map_err(|e| e.to_string())?;
    git(base, &["worktree", "add", "--detach", &wt.to_string_lossy(), "HEAD"])?;

    // 未提交的 tracked 改动不能遗漏。
    let diff = git(base, &["diff", "HEAD"])?;
    if !diff.trim().is_empty() {
        let patch = run_dir.join(format!("{node_id}-baseline.patch"));
        std::fs::write(&patch, diff).map_err(|e| e.to_string())?;
        git(&wt, &["apply", &patch.to_string_lossy()])?;
    }
    // untracked 文件复制到隔离区。
    let untracked = git(base, &["ls-files", "--others", "--exclude-standard"])?;
    for rel in untracked.lines().filter(|l| !l.trim().is_empty()) {
        let src = base.join(rel);
        let dst = wt.join(rel);
        if src.is_file() {
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::copy(&src, &dst).map_err(|e| e.to_string())?;
        }
    }
    Ok(wt)
}

/// 非 Git 任务的独立输出目录。
pub fn scratch_dir(run_dir: &Path, node_id: &str) -> PathBuf {
    run_dir.join("scratch").join(node_id)
}

pub fn cleanup(base: &Path, wt: &Path) {
    if wt.exists() {
        let _ = git(base, &["worktree", "remove", "--force", &wt.to_string_lossy()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("orch-ws-{tag}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn non_git_base_fails_isolation() {
        let dir = temp_dir("nongit");
        assert!(prepare_isolated(&dir, &dir.join("run"), "work-1").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn worktree_includes_uncommitted_and_untracked() {
        let base = temp_dir("git");
        let run = |args: &[&str]| git(&base, args).unwrap();
        run(&["init", "-q"]);
        run(&["config", "user.email", "qa@test"]);
        run(&["config", "user.name", "qa"]);
        std::fs::write(base.join("a.txt"), "v1").unwrap();
        run(&["add", "a.txt"]);
        run(&["commit", "-q", "-m", "init"]);
        // 未提交 tracked 改动 + untracked 文件。
        std::fs::write(base.join("a.txt"), "v2-uncommitted").unwrap();
        std::fs::write(base.join("new.txt"), "untracked").unwrap();
        let run_dir = base.join("run");
        let wt = prepare_isolated(&base, &run_dir, "work-1").unwrap();
        assert_eq!(std::fs::read_to_string(wt.join("a.txt")).unwrap(), "v2-uncommitted");
        assert_eq!(std::fs::read_to_string(wt.join("new.txt")).unwrap(), "untracked");
        // 用户工作区不被隔离过程改动。
        assert_eq!(std::fs::read_to_string(base.join("a.txt")).unwrap(), "v2-uncommitted");
        cleanup(&base, &wt);
        let _ = std::fs::remove_dir_all(&base);
    }
}