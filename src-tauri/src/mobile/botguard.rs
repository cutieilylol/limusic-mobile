//! Android: no BotGuard. It runs on deno_core, and V8 (rusty_v8) publishes no Android build. Every
//! spawn fails `Fatal`, which `PoTokenGenerator` latches on: from then on it hands out no tokens and
//! the orchestrator resolves through the clients that need none (VISIONOS first), the same
//! degradation the desktop build takes when its JS runtime will not start.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("botguard runtime: {0}")]
    Fatal(String),
    #[allow(dead_code)]
    #[error("botguard: {0}")]
    Transient(String),
}

pub struct Minter;

pub struct Bootstrap {
    pub minter: Minter,
    pub session_token: String,
    pub lifetime_secs: u64,
}

impl Minter {
    pub async fn spawn(_user_agent: String, _session_ident: String) -> Result<Bootstrap, Error> {
        Err(Error::Fatal("BotGuard isn't available on Android".into()))
    }

    pub async fn mint(&self, _ident: &str) -> Result<String, Error> {
        Err(Error::Fatal("BotGuard isn't available on Android".into()))
    }
}
