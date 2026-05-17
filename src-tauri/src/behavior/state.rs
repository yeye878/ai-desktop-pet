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
