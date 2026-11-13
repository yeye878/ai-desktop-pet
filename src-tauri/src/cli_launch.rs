//! External CLIs run independently of the desktop application that launched the pet.
use std::path::Path;
use tokio::process::Command;

// These are per-host transport/identity hints, not credentials or sandbox policy.
// Inheriting REGISTERED_CORE outside the packaged host breaks command execution
// and node_repl with "registered Core launch requires an installed app".
const DESKTOP_LAUNCH_ENV: &[&str] = &[
    "CODEX_WINDOWS_REGISTERED_CORE",
    "CODEX_WINDOWS_SANDBOX_PACKAGE_FAMILY",
    "CODEX_INTERNAL_ORIGINATOR_OVERRIDE",
    "CODEX_APP_TOOLS_PIPE_PATH",
    "CODEX_APP_RUNTIME_CLI_PATH",
];

pub fn command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    for variable in DESKTOP_LAUNCH_ENV {
        command.env_remove(variable);
    }
    // Keep CODEX_HOME, authentication, model/provider choices and sandbox config.
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn external_cli_drops_only_desktop_host_launch_identity() {
        let command = command(Path::new("codex"));
        let changes: Vec<_> = command.as_std().get_envs().collect();
        for key in DESKTOP_LAUNCH_ENV {
            assert!(changes.contains(&(OsStr::new(key), None)), "{key}");
        }
        for key in ["CODEX_HOME", "OPENAI_API_KEY", "DEEPSEEK_API_KEY", "CODEX_SANDBOX_NETWORK_DISABLED", "PATH"] {
            assert!(!changes.iter().any(|(name, _)| *name == OsStr::new(key)), "must preserve {key}");
        }
    }
}
