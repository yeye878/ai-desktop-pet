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
    /// 失败时附带已经输出的正文：出错前模型已经回答的部分不该随错误一起丢掉。
    pub fn with_partial_text(mut self, text: &str) -> Self {
        self.partial_text = text.to_string();
        self
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
