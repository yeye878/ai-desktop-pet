mod client;

pub use client::{
    build_system_prompt, is_cli_available, normalize_personality_id, normalize_profession_id,
    read_stream_with_activity, ClaudeAdapter, SpawnOptions, StreamParseState, DEFAULT_PERSONALITY,
    DEFAULT_PROFESSION,
};
