use anyhow::Result;
use azalea::prelude::*;
use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// Constants for connection and timing
const INIT_WAIT_ATTEMPTS: u32 = 50;
const INIT_WAIT_DELAY_MS: u64 = 100;
const GAME_STATE_WAIT_ATTEMPTS: u32 = 100;
const WORLD_SYNC_DELAY_MS: u64 = 500;

#[derive(Clone, Component)]
struct State {
    client_handle: Arc<RwLock<Option<Client>>>,
    in_game: Arc<AtomicBool>,
    chat_tx: Option<std::sync::mpsc::Sender<(Option<String>, String)>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            client_handle: Arc::new(RwLock::new(None)),
            in_game: Arc::new(AtomicBool::new(false)),
            chat_tx: None,
        }
    }
}

#[derive(Clone, Default)]
pub struct TestBot {
    client: Option<Arc<RwLock<Option<Client>>>>,
    in_game: Option<Arc<AtomicBool>>,
    chat_rx: Option<Arc<parking_lot::Mutex<std::sync::mpsc::Receiver<(Option<String>, String)>>>>,
}

impl TestBot {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a reference to the client, or error if not connected
    fn get_client(&self) -> Result<parking_lot::RwLockReadGuard<'_, Option<Client>>> {
        self.client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Bot not connected"))
            .map(|handle| handle.read())
    }

    pub fn connect(&mut self, server: &str) -> Result<()> {
        let account = Account::offline("flintmc_testbot");

        tracing::info!("Connecting to server: {}", server);

        // Create chat channel
        let (chat_tx, chat_rx) = std::sync::mpsc::channel();

        let state = State {
            chat_tx: Some(chat_tx),
            ..Default::default()
        };
        let client_handle = state.client_handle.clone();
        let in_game = state.in_game.clone();

        // Spawn the bot in a background thread with LocalSet (required by new azalea version)
        let server_owned = server.to_string();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime");

            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, async move {
                async fn handler(bot: Client, event: Event, state: State) -> Result<()> {
                    match event {
                        Event::Init => {
                            *state.client_handle.write() = Some(bot.clone());
                            tracing::info!("Bot initialized");
                        }
                        Event::Login => {
                            // Login event means we're fully in the game state
                            state.in_game.store(true, Ordering::SeqCst);
                            tracing::info!("Bot in game state");
                        }
                        Event::Chat(m) => {
                            // Extract the message content
                            let message = m.message().to_string();
                            // Try to get sender name (best effort)
                            // Fallback: parse "<Name>"
                            let sender = if message.starts_with('<') {
                                message.find('>').map(|end| message[1..end].to_string())
                            } else {
                                None
                            };

                            if let Some(ref tx) = state.chat_tx {
                                let _ = tx.send((sender, message));
                            }
                        }
                        _ => {}
                    }
                    Ok(())
                }

                let result = ClientBuilder::new()
                    .set_handler(handler)
                    .set_state(state)
                    .start(account, server_owned.as_str())
                    .await;

                if let AppExit::Error(e) = result {
                    tracing::error!("Bot connection error: {}", e);
                }
            });
        });

        // Wait for client to initialize
        for _ in 0..INIT_WAIT_ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(INIT_WAIT_DELAY_MS));
            if client_handle.read().is_some() {
                break;
            }
        }

        if client_handle.read().is_none() {
            anyhow::bail!("Failed to initialize bot connection");
        }

        // Wait for bot to be in game state
        tracing::info!("Waiting for bot to enter game state...");
        for _ in 0..GAME_STATE_WAIT_ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(INIT_WAIT_DELAY_MS));
            if in_game.load(Ordering::SeqCst) {
                break;
            }
        }

        if !in_game.load(Ordering::SeqCst) {
            anyhow::bail!("Bot failed to enter game state within timeout");
        }

        self.client = Some(client_handle);
        self.in_game = Some(in_game);
        self.chat_rx = Some(Arc::new(parking_lot::Mutex::new(chat_rx)));
        tracing::info!("Connected successfully and in game state");

        // Give a small amount of extra time for world data to sync
        std::thread::sleep(std::time::Duration::from_millis(WORLD_SYNC_DELAY_MS));

        Ok(())
    }

    /// Wait for a chat message with timeout
    pub fn recv_chat_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Option<(Option<String>, String)> {
        if let Some(ref rx_mutex) = self.chat_rx {
            let rx = rx_mutex.lock();
            rx.recv_timeout(timeout).ok()
        } else {
            None
        }
    }

    pub fn send_command(&self, command: &str) -> Result<()> {
        let client_guard = self.get_client()?;
        let client = client_guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Bot not initialized"))?;

        // Add "/" prefix if not present
        let command_with_slash = if command.starts_with('/') {
            command.to_string()
        } else {
            format!("/{}", command)
        };
        tracing::debug!("Sending command: {}", command_with_slash);
        client.chat(&command_with_slash);
        Ok(())
    }

    pub fn get_block(&self, pos: [i32; 3]) -> Result<Option<String>> {
        let client_guard = self.get_client()?;
        let client = client_guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Bot not initialized"))?;

        let block_pos = azalea::BlockPos::new(pos[0], pos[1], pos[2]);
        if let Ok(world_lock) = client.world() {
            let world = world_lock.read();
            let block_state = world.get_block_state(block_pos);

            if let Some(state) = block_state {
                // Return block state as debug string
                let state_str = format!("{:?}", state);
                Ok(Some(state_str))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    /// Get the bot's current position
    pub fn get_position(&self) -> Result<[i32; 3]> {
        let client_guard = self.get_client()?;
        let client = client_guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Bot not initialized"))?;

        if let Ok(pos) = client.position() {
            return Ok([pos.x as i32, pos.y as i32, pos.z as i32]);
        }
        Ok([0, 0, 0])
    }
}
