//! Incremental SHA-256 helpers shared by native CakeSplitter workflows.

use sha2::{Digest, Sha256};

#[derive(Default)]
pub struct Sha256State(Sha256);

impl Sha256State {
    pub fn new() -> Self {
        Self(Sha256::new())
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    pub fn finish(self) -> String {
        hex::encode(self.0.finalize())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_incrementally() {
        let mut state = Sha256State::new();
        state.update(b"split");
        state.update(b"thecake");
        assert_eq!(
            state.finish(),
            "ce63f61eb97f2a8e766fdae714070ab09295cb1ebc18ba9c2236eef3bef4b5de"
        );
    }
}
