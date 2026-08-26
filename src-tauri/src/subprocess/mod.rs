pub mod manager;

pub use manager::{
    kill_subprocess, list_subprocesses, send_stdin, spawn_subprocess, ProcessInfo, SpawnParams,
    SubprocessManager,
};
#[allow(unused_imports)]
pub use manager::{EVENT_SUBPROC_EXIT, EVENT_SUBPROC_STDERR, EVENT_SUBPROC_STDOUT};
