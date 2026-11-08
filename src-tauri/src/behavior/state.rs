use super::mood::Mood;
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum PetState {
    Idle,
    Listening,
    Thinking,
    Speaking,
    Working,
    Sleeping,
    Happy,
    Confused,
    Waving,
    Dragging,
    Hungry,
    Stuffed,
    Refusing,
}

impl PetState {
    pub fn as_str(&self) -> &'static str {
        match self {
            PetState::Idle => "idle",
            PetState::Listening => "listening",
            PetState::Thinking => "thinking",
            PetState::Speaking => "speaking",
            PetState::Working => "working",
            PetState::Sleeping => "sleeping",
            PetState::Happy => "happy",
            PetState::Confused => "confused",
            PetState::Waving => "waving",
            PetState::Dragging => "dragging",
            PetState::Hungry => "hungry",
            PetState::Stuffed => "stuffed",
            PetState::Refusing => "refusing",
        }
    }
}

pub struct BehaviorEngine {
    pub state: PetState,
    pub mood: Mood,
    pub idle_ticks: u64,
    pub state_ticks: u64,
}

impl BehaviorEngine {
    pub fn new() -> Self {
        Self {
            state: PetState::Waving, // 开机打招呼
            mood: Mood::new(),
            idle_ticks: 0,
            state_ticks: 0,
        }
    }

    pub fn set_state(&mut self, state: PetState) {
        if self.state != state {
            self.state = state;
            self.state_ticks = 0;
        }
    }

    /// 每秒调用一次，自动更新状态
    pub fn tick(&mut self) {
        self.state_ticks += 1;

        match &self.state {
            PetState::Waving => {
                if self.state_ticks >= 3 {
                    self.set_state(PetState::Idle);
                }
            }
            PetState::Speaking | PetState::Confused => {
                if self.state_ticks >= 5 {
                    self.set_state(PetState::Idle);
                }
            }
            PetState::Happy => {
                if self.state_ticks >= 4 {
                    self.set_state(PetState::Idle);
                }
            }
            PetState::Stuffed => {
                if self.state_ticks >= 4 {
                    self.set_state(PetState::Idle);
                }
            }
            PetState::Refusing => {
                if self.state_ticks >= 3 {
                    self.set_state(PetState::Idle);
                }
            }
            PetState::Hungry => {
                if self.state_ticks >= 10 {
                    self.set_state(PetState::Idle);
                }
            }
            // 以下三态由外部显式驱动（语音面板 / 拖拽 / 工具执行），正常结束时会有人负责切走；
            // 这里的超时只做兜底，防止对应窗口崩溃或事件丢失把桌宠永久卡在某个状态。
            PetState::Listening => {
                if self.state_ticks >= 120 {
                    self.set_state(PetState::Idle);
                }
            }
            PetState::Dragging => {
                if self.state_ticks >= 60 {
                    self.set_state(PetState::Idle);
                }
            }
            PetState::Working => {
                if self.state_ticks >= 1800 {
                    self.set_state(PetState::Idle);
                }
            }
            PetState::Idle => {
                self.idle_ticks += 1;
                self.mood.update(None, Some(-0.02));
                if self.idle_ticks >= 600 {
                    self.set_state(PetState::Sleeping);
                    self.idle_ticks = 0;
                }
            }
            PetState::Sleeping => {
                self.mood.update(None, Some(0.1));
                if self.mood.energy >= 100.0 {
                    self.set_state(PetState::Idle);
                }
            }
            _ => {}
        }
    }

    pub fn on_user_interaction(&mut self) {
        self.idle_ticks = 0;
        if self.state == PetState::Sleeping {
            self.set_state(PetState::Waving);
        }
        self.mood.update(Some(0.02), Some(-0.5));
    }

    pub fn get_state_json(&self) -> Value {
        json!({
            "state": self.state.as_str(),
            "happiness": self.mood.happiness,
            "energy": self.mood.energy,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 十三态必须都能被外部设置并如实报出自己的名字（这是作品方案里"13 态"的底账）。
    #[test]
    fn all_thirteen_states_round_trip() {
        let states = [
            PetState::Idle,
            PetState::Listening,
            PetState::Thinking,
            PetState::Speaking,
            PetState::Working,
            PetState::Sleeping,
            PetState::Happy,
            PetState::Confused,
            PetState::Waving,
            PetState::Dragging,
            PetState::Hungry,
            PetState::Stuffed,
            PetState::Refusing,
        ];
        assert_eq!(states.len(), 13);

        let mut engine = BehaviorEngine::new();
        for state in states {
            engine.set_state(state.clone());
            assert_eq!(engine.state, state);
            assert_eq!(engine.get_state_json()["state"], json!(state.as_str()));
        }
    }

    /// 外部驱动的三态必须能自己超时退回 idle，避免窗口崩溃后桌宠永久卡住。
    #[test]
    fn externally_driven_states_time_out_to_idle() {
        for (state, limit) in [
            (PetState::Listening, 120_u64),
            (PetState::Dragging, 60),
            (PetState::Working, 1800),
        ] {
            let mut engine = BehaviorEngine::new();
            engine.set_state(state.clone());
            for _ in 0..limit - 1 {
                engine.tick();
                assert_eq!(engine.state, state, "{} 提前退出了", state.as_str());
            }
            engine.tick();
            assert_eq!(engine.state, PetState::Idle, "{} 没有超时", state.as_str());
        }
    }

    /// working 的兜底时间必须明显长于一次普通工具调用，否则长任务跑一半就会被打断。
    #[test]
    fn working_timeout_is_longer_than_a_normal_tool_call() {
        let mut engine = BehaviorEngine::new();
        engine.set_state(PetState::Working);
        for _ in 0..600 {
            engine.tick();
        }
        assert_eq!(engine.state, PetState::Working);
    }

    /// 回到 idle 后 idle_ticks 必须继续累积，否则久坐入睡的彩蛋会失效。
    #[test]
    fn idle_still_accumulates_after_controlled_states() {
        let mut engine = BehaviorEngine::new();
        engine.set_state(PetState::Listening);
        for _ in 0..120 {
            engine.tick();
        }
        assert_eq!(engine.state, PetState::Idle);
        for _ in 0..599 {
            engine.tick();
        }
        assert_eq!(engine.state, PetState::Idle);
        engine.tick();
        assert_eq!(engine.state, PetState::Sleeping);
    }
}
