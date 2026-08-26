use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};
use std::{
    fs::File,
    io::BufReader,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

pub struct NativeAudioPlayer {
    current: Mutex<Option<PlaybackSession>>,
    next_id: AtomicU64,
}

struct PlaybackSession {
    id: u64,
    _stream: MixerDeviceSink,
    player: Arc<Player>,
}

impl NativeAudioPlayer {
    pub fn new() -> Self {
        Self {
            current: Mutex::new(None),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn play_wav_file(&self, path: impl AsRef<Path>) -> Result<(u64, Arc<Player>), String> {
        self.stop();

        let path = path.as_ref();
        let file = File::open(path).map_err(|e| format!("打开 TTS 音频失败: {e}"))?;
        let source = Decoder::new_wav(BufReader::new(file))
            .map_err(|e| format!("解码 TTS WAV 失败: {e}"))?;
        let mut stream =
            DeviceSinkBuilder::open_default_sink().map_err(|e| format!("打开音频设备失败: {e}"))?;
        stream.log_on_drop(false);

        let player = Arc::new(Player::connect_new(stream.mixer()));
        player.append(source);

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut current = self
            .current
            .lock()
            .map_err(|_| "TTS 播放状态锁定失败".to_string())?;
        *current = Some(PlaybackSession {
            id,
            _stream: stream,
            player: Arc::clone(&player),
        });
        Ok((id, player))
    }

    pub fn stop(&self) {
        let session = self
            .current
            .lock()
            .ok()
            .and_then(|mut current| current.take());
        if let Some(session) = session {
            session.player.stop();
        }
    }

    pub fn clear_if_current(&self, id: u64) {
        if let Ok(mut current) = self.current.lock() {
            if current.as_ref().is_some_and(|session| session.id == id) {
                current.take();
            }
        }
    }
}

impl Default for NativeAudioPlayer {
    fn default() -> Self {
        Self::new()
    }
}
