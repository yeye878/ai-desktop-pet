#[derive(Debug)]
pub struct TurnOutput {
    pub text: String,
    pub thinking: String,
}

#[derive(Debug)]
pub struct TurnError {
    pub message: String,
    pub thinking: String,
    pub partial_text: String,
    pub aborted: bool,
}

impl TurnError {
    pub fn failed(message: impl Into<String>, thinking: &str) -> Self {
        Self {
            message: message.into(),
            thinking: thinking.into(),
            partial_text: String::new(),
            aborted: false,
        }
    }
    pub fn aborted(text: &str, thinking: &str) -> Self {
        Self {
            message: "已停止协同".into(),
            thinking: thinking.into(),
            partial_text: text.into(),
            aborted: true,
        }
    }
}
