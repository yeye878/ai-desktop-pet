mod client;

pub use client::{
    build_system_prompt, normalize_personality_id, normalize_profession_id, read_stream_with_activity,
    ClaudeAdapter, StreamParseState, DEFAULT_PERSONALITY, DEFAULT_PROFESSION,
};
