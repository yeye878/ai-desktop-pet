use sysinfo::System;

pub struct SystemMonitor {
    sys: System,
}

impl SystemMonitor {
    pub fn new() -> Self {
        Self {
            sys: System::new_all(),
        }
    }

    pub fn refresh(&mut self) {
        self.sys.refresh_all();
    }

    pub fn cpu_usage(&self) -> f32 {
        self.sys.global_cpu_usage()
    }

    pub fn memory_usage_percent(&self) -> f32 {
        let total = self.sys.total_memory() as f32;
        let used = self.sys.used_memory() as f32;
        if total > 0.0 {
            (used / total) * 100.0
        } else {
            0.0
        }
    }
}
