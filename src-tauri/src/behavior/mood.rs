pub struct Mood {
    pub happiness: f32, // -1.0 ~ 1.0
    pub energy: f32,    // 0 ~ 100
}

impl Mood {
    pub fn new() -> Self {
        Self {
            happiness: 0.5,
            energy: 80.0,
        }
    }

    pub fn update(&mut self, dh: Option<f32>, de: Option<f32>) {
        if let Some(h) = dh {
            self.happiness = (self.happiness + h).clamp(-1.0, 1.0);
        }
        if let Some(e) = de {
            self.energy = (self.energy + e).clamp(0.0, 100.0);
        }
    }
}
