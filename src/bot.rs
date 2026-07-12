use anyhow::Result;
use azalea::app::{App, Plugin, Update};
use azalea::ecs::schedule::IntoScheduleConfigs;
use azalea::prelude::*;
use flint_core::test_spec::{GameMode, Item, PlayerSlot};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

// Constants for connection and timing
const INIT_WAIT_ATTEMPTS: u32 = 50;
const INIT_WAIT_DELAY_MS: u64 = 100;
const GAME_STATE_WAIT_ATTEMPTS: u32 = 100;
const STATE_SYNC_TIMEOUT_MS: u64 = 2_000;
const STATE_SYNC_POLL_MS: u64 = 5;
// Must be configured during Event::Init, before Azalea allocates PartialWorld.
// Vanilla servers support at most 32 chunks and may clamp this request lower.
const CLIENT_VIEW_DISTANCE: u8 = 32;

type ChatReceiver = std::sync::mpsc::Receiver<(Option<String>, String)>;
type AckReceiver = std::sync::mpsc::Receiver<String>;
type UpdateReceiver = std::sync::mpsc::Receiver<()>;

struct TestBotRuntimePlugin {
    update_tx: std::sync::mpsc::SyncSender<()>,
}

impl Plugin for TestBotRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            azalea::core::tick::GameTick,
            azalea::physics::PhysicsSystems.run_if(|| false),
        );
        let update_tx = self.update_tx.clone();
        app.add_systems(Update, move || {
            let _ = update_tx.try_send(());
        });
    }
}

#[derive(Clone, Default)]
struct ActivePlayer {
    owner: Option<u64>,
    slots: HashMap<PlayerSlot, Item>,
    selected_hotbar: Option<u8>,
    position: Option<[f64; 3]>,
    rotation: Option<[f32; 2]>,
    game_mode: Option<GameMode>,
}

#[derive(Clone, Component)]
struct State {
    client_handle: Arc<RwLock<Option<Client>>>,
    in_game: Arc<AtomicBool>,
    chat_tx: Option<std::sync::mpsc::Sender<(Option<String>, String)>>,
    ack_tx: Option<std::sync::mpsc::Sender<String>>,
    world_ready_tx: Option<std::sync::mpsc::SyncSender<()>>,
    view_distance: Arc<AtomicU32>,
    simulation_distance: Arc<AtomicU32>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            client_handle: Arc::new(RwLock::new(None)),
            in_game: Arc::new(AtomicBool::new(false)),
            chat_tx: None,
            ack_tx: None,
            world_ready_tx: None,
            view_distance: Arc::new(AtomicU32::new(0)),
            simulation_distance: Arc::new(AtomicU32::new(0)),
        }
    }
}

#[derive(Clone)]
pub struct TestBot {
    client: Option<Arc<RwLock<Option<Client>>>>,
    in_game: Option<Arc<AtomicBool>>,
    chat_rx: Option<Arc<parking_lot::Mutex<ChatReceiver>>>,
    ack_rx: Option<Arc<parking_lot::Mutex<AckReceiver>>>,
    update_rx: Option<Arc<parking_lot::Mutex<UpdateReceiver>>>,
    next_inventory_owner: Arc<AtomicU64>,
    next_command_ack: Arc<AtomicU64>,
    active_player: Arc<parking_lot::Mutex<ActivePlayer>>,
    view_distance: Arc<AtomicU32>,
    simulation_distance: Arc<AtomicU32>,
}

impl Default for TestBot {
    fn default() -> Self {
        Self {
            client: None,
            in_game: None,
            chat_rx: None,
            ack_rx: None,
            update_rx: None,
            next_inventory_owner: Arc::new(AtomicU64::new(1)),
            next_command_ack: Arc::new(AtomicU64::new(1)),
            active_player: Arc::new(parking_lot::Mutex::new(ActivePlayer::default())),
            view_distance: Arc::new(AtomicU32::new(0)),
            simulation_distance: Arc::new(AtomicU32::new(0)),
        }
    }
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
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        let (update_tx, update_rx) = std::sync::mpsc::sync_channel(1);
        let (world_ready_tx, world_ready_rx) = std::sync::mpsc::sync_channel(1);

        let state = State {
            chat_tx: Some(chat_tx),
            ack_tx: Some(ack_tx),
            world_ready_tx: Some(world_ready_tx),
            ..Default::default()
        };
        let client_handle = state.client_handle.clone();
        let in_game = state.in_game.clone();
        let view_distance = state.view_distance.clone();
        let simulation_distance = state.simulation_distance.clone();

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
                            bot.set_client_information(azalea::ClientInformation {
                                view_distance: CLIENT_VIEW_DISTANCE,
                                ..Default::default()
                            })?;
                            *state.client_handle.write() = Some(bot.clone());
                            tracing::info!("Bot initialized");
                        }
                        Event::Login => {
                            // Login event means we're fully in the game state
                            state.in_game.store(true, Ordering::SeqCst);
                            tracing::info!("Bot in game state");
                        }
                        Event::Spawn => {
                            if let Some(tx) = &state.world_ready_tx {
                                let _ = tx.try_send(());
                            }
                            tracing::info!("Bot world spawned");
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

                            if message.contains("__flintmc_ack_") {
                                if let Some(tx) = &state.ack_tx {
                                    let _ = tx.send(message);
                                }
                            } else if let Some(ref tx) = state.chat_tx {
                                let _ = tx.send((sender, message));
                            }
                        }
                        Event::Packet(packet) => {
                            use azalea::protocol::packets::game::ClientboundGamePacket;
                            match &*packet {
                                ClientboundGamePacket::Login(packet) => {
                                    let effective =
                                        packet.chunk_radius.min(packet.simulation_distance);
                                    bot.set_client_information(azalea::ClientInformation {
                                        view_distance: effective.min(u8::MAX.into()) as u8,
                                        ..Default::default()
                                    })?;
                                    state.view_distance.store(effective, Ordering::SeqCst);
                                    state
                                        .simulation_distance
                                        .store(packet.simulation_distance, Ordering::SeqCst);
                                }
                                ClientboundGamePacket::SetChunkCacheRadius(packet) => {
                                    state.view_distance.store(packet.radius, Ordering::SeqCst);
                                }
                                ClientboundGamePacket::SetSimulationDistance(packet) => {
                                    state
                                        .simulation_distance
                                        .store(packet.simulation_distance, Ordering::SeqCst);
                                    let effective = state
                                        .view_distance
                                        .load(Ordering::SeqCst)
                                        .min(packet.simulation_distance);
                                    bot.set_client_information(azalea::ClientInformation {
                                        view_distance: effective.min(u8::MAX.into()) as u8,
                                        ..Default::default()
                                    })?;
                                    state.view_distance.store(effective, Ordering::SeqCst);
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                    Ok(())
                }

                let result = ClientBuilder::new()
                    .add_plugins(TestBotRuntimePlugin { update_tx })
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
        self.ack_rx = Some(Arc::new(parking_lot::Mutex::new(ack_rx)));
        self.update_rx = Some(Arc::new(parking_lot::Mutex::new(update_rx)));
        self.view_distance = view_distance;
        self.simulation_distance = simulation_distance;
        tracing::info!("Connected successfully and in game state");

        world_ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| anyhow::anyhow!("Bot world did not spawn within timeout"))?;
        self.reset_to_test_origin()?;

        Ok(())
    }

    pub fn reset_to_test_origin(&self) -> Result<()> {
        let client = {
            let client_guard = self.get_client()?;
            client_guard
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Bot not initialized"))?
                .clone()
        };
        let mut updates = client.get_update_broadcaster();
        self.send_command("execute in minecraft:overworld run setblock 0 63 0 minecraft:bedrock")?;
        self.send_command("execute in minecraft:overworld run tp flintmc_testbot 0.5 64 0.5 0 0")?;
        self.keep_airborne()?;
        self.wait_until("Overworld test origin", || {
            let in_overworld = client
                .world_name()
                .is_ok_and(|world| world.to_string() == "minecraft:overworld");
            let at_origin = client.position().is_ok_and(|position| {
                (position.x - 0.5).abs() < 0.01 && (position.z - 0.5).abs() < 0.01
            });
            in_overworld && at_origin
        })
        .map_err(|error| {
            anyhow::anyhow!(
                "{error}; current world={:?}, position={:?}",
                client.world_name(),
                client.position()
            )
        })?;
        let _ = updates.blocking_recv();
        let _ = updates.blocking_recv();
        *self.active_player.lock() = ActivePlayer::default();
        Ok(())
    }

    /// Park the physical bot at the shared focus point and discard any active virtual
    /// player ownership. The next player action will restore that player's full state.
    pub fn park_at(&self, position: [f64; 3]) -> Result<()> {
        self.keep_airborne()?;
        self.teleport(position, Some([0.0, 0.0]))?;
        *self.active_player.lock() = ActivePlayer::default();
        Ok(())
    }

    pub fn keep_airborne(&self) -> Result<()> {
        let client = {
            let client_guard = self.get_client()?;
            client_guard
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Bot not initialized"))?
                .clone()
        };
        client.query_self::<&mut azalea::entity::PlayerAbilities, _>(|mut abilities| {
            abilities.can_fly = true;
            abilities.flying = true;
        })?;
        client.write_packet(
            azalea::protocol::packets::game::s_player_abilities::ServerboundPlayerAbilities {
                is_flying: true,
            },
        );
        // The acknowledgement packet is sent after the abilities packet on the same
        // connection, so receiving it proves the server has processed flying=true.
        self.send_command_synced("execute if entity flintmc_testbot run return 1")?;
        Ok(())
    }

    pub fn effective_chunk_distance(&self) -> Result<u32> {
        let view = self.view_distance.load(Ordering::SeqCst);
        let simulation = self.simulation_distance.load(Ordering::SeqCst);
        if view == 0 || simulation == 0 {
            anyhow::bail!(
                "server did not advertise view/simulation distance (view={view}, simulation={simulation})"
            );
        }
        Ok(view.min(simulation))
    }

    pub fn detected_distances(&self) -> (u32, u32) {
        (
            self.view_distance.load(Ordering::SeqCst),
            self.simulation_distance.load(Ordering::SeqCst),
        )
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

    /// Send a command and wait until the server has processed it. Commands from one
    /// connection are ordered, so receiving the marker also acknowledges every command
    /// sent before it without relying on an arbitrary delay.
    pub fn send_command_synced(&self, command: &str) -> Result<()> {
        self.send_command(command)?;
        let id = self.next_command_ack.fetch_add(1, Ordering::Relaxed);
        let marker = format!("__flintmc_ack_{id}__");
        self.send_command(&format!(
            "tellraw flintmc_testbot {{\"text\":\"{marker}\"}}"
        ))?;

        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(STATE_SYNC_TIMEOUT_MS);
        while std::time::Instant::now() < deadline {
            let Some(ack_rx) = &self.ack_rx else {
                anyhow::bail!("command acknowledgement channel is unavailable");
            };
            if ack_rx
                .lock()
                .recv_timeout(std::time::Duration::from_millis(STATE_SYNC_POLL_MS))
                .is_ok_and(|message| message.contains(&marker))
            {
                return Ok(());
            }
        }
        anyhow::bail!("timed out waiting for command acknowledgement: {command}")
    }

    /// Fence server world changes against Azalea's packet processing. The marker is
    /// emitted after earlier commands/ticks. After observing it, wait for a subsequent
    /// Azalea ECS update so packet-driven world mutations are visible to readers.
    pub fn sync_client_world(&self) -> Result<()> {
        self.send_command_synced("execute if entity flintmc_testbot run return 1")?;
        let Some(update_rx) = &self.update_rx else {
            anyhow::bail!("Azalea update channel is unavailable");
        };
        let update_rx = update_rx.lock();
        while update_rx.try_recv().is_ok() {}
        update_rx
            .recv_timeout(std::time::Duration::from_millis(STATE_SYNC_TIMEOUT_MS))
            .map_err(|error| anyhow::anyhow!("failed waiting for Azalea world update: {error}"))?;
        Ok(())
    }

    pub fn teleport(&self, pos: [f64; 3], rot: Option<[f32; 2]>) -> Result<()> {
        let client = {
            let client_guard = self.get_client()?;
            client_guard
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Bot not initialized"))?
                .clone()
        };
        let mut updates = client.get_update_broadcaster();
        let command = match rot {
            Some([yaw, pitch]) => format!(
                "execute in minecraft:overworld run tp flintmc_testbot {} {} {} {} {}",
                pos[0], pos[1], pos[2], yaw, pitch
            ),
            None => format!(
                "execute in minecraft:overworld run tp flintmc_testbot {} {} {}",
                pos[0], pos[1], pos[2]
            ),
        };
        self.send_command(&command)?;

        if let Some([yaw, pitch]) = rot {
            client.set_direction(yaw, pitch)?;
        }

        self.wait_until("teleport", || {
            client
                .world_name()
                .is_ok_and(|world| world.to_string() == "minecraft:overworld")
                && client.position().is_ok_and(|actual| {
                    (actual.x - pos[0]).abs() < 0.01
                        && (actual.y - pos[1]).abs() < 0.75
                        && (actual.z - pos[2]).abs() < 0.01
                })
        })?;
        let _ = updates.blocking_recv();
        let _ = updates.blocking_recv();
        Ok(())
    }

    pub fn interact(&self) -> Result<()> {
        let client = {
            let client_guard = self.get_client()?;
            client_guard
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Bot not initialized"))?
                .clone()
        };
        let mut updates = client.get_update_broadcaster();
        let mut ticks = client.get_tick_broadcaster();
        let block_hit = client
            .hit_result()?
            .as_block_hit_result_if_not_miss()
            .cloned();
        if let Some(hit) = block_hit {
            client.block_interact(hit.block_pos);
        } else {
            client.start_use_item();
        }
        let _ = updates.blocking_recv();
        let _ = updates.blocking_recv();
        let _ = ticks.blocking_recv();
        let _ = ticks.blocking_recv();
        Ok(())
    }

    pub fn select_hotbar(&self, slot: u8) -> Result<()> {
        if !(1..=9).contains(&slot) {
            anyhow::bail!("hotbar slot must be in the range 1..=9");
        }
        let client = {
            let client_guard = self.get_client()?;
            client_guard
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Bot not initialized"))?
                .clone()
        };
        let mut ticks = client.get_tick_broadcaster();
        client.set_selected_hotbar_slot(slot - 1);
        self.wait_until("hotbar selection", || {
            client
                .selected_hotbar_slot()
                .is_ok_and(|selected| selected == slot - 1)
        })?;
        let _ = ticks.blocking_recv();
        let _ = ticks.blocking_recv();
        Ok(())
    }

    pub fn allocate_inventory_owner(&self) -> u64 {
        self.next_inventory_owner.fetch_add(1, Ordering::Relaxed)
    }

    pub fn restore_player(
        &self,
        owner: u64,
        slots: &HashMap<PlayerSlot, Item>,
        selected_hotbar: u8,
        position: Option<[f64; 3]>,
        rotation: Option<[f32; 2]>,
        game_mode: GameMode,
    ) -> Result<()> {
        let mut active = self.active_player.lock();
        if active.owner == Some(owner)
            && active.slots == *slots
            && active.selected_hotbar == Some(selected_hotbar)
            && active.position == position
            && active.rotation == rotation
            && active.game_mode == Some(game_mode)
        {
            return Ok(());
        }

        for slot in all_player_slots() {
            let current = active.slots.get(&slot);
            let desired = slots.get(&slot);
            if active.owner.is_none() || current != desired {
                let slot_name = slot_to_minecraft_name(slot);
                let command = match desired {
                    Some(item) => format!(
                        "item replace entity flintmc_testbot {} with {} {}",
                        slot_name, item.id, item.count
                    ),
                    None => format!("item replace entity flintmc_testbot {} with air", slot_name),
                };
                self.send_command(&command)?;
            }
        }

        if active.selected_hotbar != Some(selected_hotbar) {
            self.select_hotbar(selected_hotbar)?;
        }
        if active.game_mode != Some(game_mode) {
            let mode = match game_mode {
                GameMode::Survival => "survival",
                GameMode::Creative => "creative",
                GameMode::Adventure => "adventure",
                GameMode::Spectator => "spectator",
            };
            self.send_command_synced(&format!("gamemode {mode} flintmc_testbot"))?;
        }
        self.keep_airborne()?;
        match position {
            Some(pos)
                if active.position != position
                    || active.rotation != rotation
                    || active.owner != Some(owner) =>
            {
                self.teleport(pos, rotation)?;
            }
            None if active.position.is_some() || active.owner != Some(owner) => {
                self.teleport([0.5, 64.0, 0.5], Some([0.0, 0.0]))?;
            }
            _ => {}
        }
        self.wait_for_inventory(slots)?;
        active.owner = Some(owner);
        active.slots = slots.clone();
        active.selected_hotbar = Some(selected_hotbar);
        active.position = position;
        active.rotation = rotation;
        active.game_mode = Some(game_mode);
        Ok(())
    }

    pub fn record_player(
        &self,
        owner: u64,
        slots: &HashMap<PlayerSlot, Item>,
        selected_hotbar: u8,
        position: Option<[f64; 3]>,
        rotation: Option<[f32; 2]>,
        game_mode: GameMode,
    ) {
        let mut active = self.active_player.lock();
        active.owner = Some(owner);
        active.slots = slots.clone();
        active.selected_hotbar = Some(selected_hotbar);
        active.position = position;
        active.rotation = rotation;
        active.game_mode = Some(game_mode);
    }

    pub fn wait_for_inventory(&self, slots: &HashMap<PlayerSlot, Item>) -> Result<()> {
        self.wait_until("inventory synchronization", || {
            let Ok(client_guard) = self.get_client() else {
                return false;
            };
            let Some(client) = client_guard.as_ref() else {
                return false;
            };
            let Ok(menu) = client.menu() else {
                return false;
            };
            let Some(player) = menu.try_as_player() else {
                return false;
            };

            all_player_slots().into_iter().all(|slot| {
                let actual = match slot {
                    PlayerSlot::Hotbar1 => &player.inventory[27],
                    PlayerSlot::Hotbar2 => &player.inventory[28],
                    PlayerSlot::Hotbar3 => &player.inventory[29],
                    PlayerSlot::Hotbar4 => &player.inventory[30],
                    PlayerSlot::Hotbar5 => &player.inventory[31],
                    PlayerSlot::Hotbar6 => &player.inventory[32],
                    PlayerSlot::Hotbar7 => &player.inventory[33],
                    PlayerSlot::Hotbar8 => &player.inventory[34],
                    PlayerSlot::Hotbar9 => &player.inventory[35],
                    PlayerSlot::OffHand => &player.offhand,
                    PlayerSlot::Helmet => &player.armor[0],
                    PlayerSlot::Chestplate => &player.armor[1],
                    PlayerSlot::Leggings => &player.armor[2],
                    PlayerSlot::Boots => &player.armor[3],
                };
                match slots.get(&slot) {
                    Some(expected) => {
                        actual.kind().to_str() == normalized_item_id(&expected.id)
                            && actual.count() == i32::from(expected.count)
                    }
                    None => actual.is_empty(),
                }
            })
        })
    }

    /// Read a player inventory slot from the state reported by the server.
    pub fn inventory_slot(&self, slot: PlayerSlot) -> Result<Option<Item>> {
        let client_guard = self.get_client()?;
        let client = client_guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Bot not initialized"))?;
        let menu = client.menu()?;
        let player = menu
            .try_as_player()
            .ok_or_else(|| anyhow::anyhow!("Bot does not have a player inventory open"))?;
        let actual = match slot {
            PlayerSlot::Hotbar1 => &player.inventory[27],
            PlayerSlot::Hotbar2 => &player.inventory[28],
            PlayerSlot::Hotbar3 => &player.inventory[29],
            PlayerSlot::Hotbar4 => &player.inventory[30],
            PlayerSlot::Hotbar5 => &player.inventory[31],
            PlayerSlot::Hotbar6 => &player.inventory[32],
            PlayerSlot::Hotbar7 => &player.inventory[33],
            PlayerSlot::Hotbar8 => &player.inventory[34],
            PlayerSlot::Hotbar9 => &player.inventory[35],
            PlayerSlot::OffHand => &player.offhand,
            PlayerSlot::Helmet => &player.armor[0],
            PlayerSlot::Chestplate => &player.armor[1],
            PlayerSlot::Leggings => &player.armor[2],
            PlayerSlot::Boots => &player.armor[3],
        };
        if actual.is_empty() {
            return Ok(None);
        }
        let count = u8::try_from(actual.count())
            .map_err(|_| anyhow::anyhow!("invalid inventory count: {}", actual.count()))?;
        Ok(Some(Item::with_count(actual.kind().to_str(), count)))
    }

    pub(crate) fn wait_until(
        &self,
        operation: &str,
        mut predicate: impl FnMut() -> bool,
    ) -> Result<()> {
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(STATE_SYNC_TIMEOUT_MS);
        while std::time::Instant::now() < deadline {
            if predicate() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(STATE_SYNC_POLL_MS));
        }
        anyhow::bail!("Timed out waiting for {operation}");
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

pub fn slot_to_minecraft_name(slot: PlayerSlot) -> &'static str {
    match slot {
        PlayerSlot::Hotbar1 => "hotbar.0",
        PlayerSlot::Hotbar2 => "hotbar.1",
        PlayerSlot::Hotbar3 => "hotbar.2",
        PlayerSlot::Hotbar4 => "hotbar.3",
        PlayerSlot::Hotbar5 => "hotbar.4",
        PlayerSlot::Hotbar6 => "hotbar.5",
        PlayerSlot::Hotbar7 => "hotbar.6",
        PlayerSlot::Hotbar8 => "hotbar.7",
        PlayerSlot::Hotbar9 => "hotbar.8",
        PlayerSlot::OffHand => "weapon.offhand",
        PlayerSlot::Helmet => "armor.head",
        PlayerSlot::Chestplate => "armor.chest",
        PlayerSlot::Leggings => "armor.legs",
        PlayerSlot::Boots => "armor.feet",
    }
}

fn all_player_slots() -> [PlayerSlot; 14] {
    [
        PlayerSlot::Hotbar1,
        PlayerSlot::Hotbar2,
        PlayerSlot::Hotbar3,
        PlayerSlot::Hotbar4,
        PlayerSlot::Hotbar5,
        PlayerSlot::Hotbar6,
        PlayerSlot::Hotbar7,
        PlayerSlot::Hotbar8,
        PlayerSlot::Hotbar9,
        PlayerSlot::OffHand,
        PlayerSlot::Helmet,
        PlayerSlot::Chestplate,
        PlayerSlot::Leggings,
        PlayerSlot::Boots,
    ]
}

fn normalized_item_id(id: &str) -> String {
    if id.contains(':') {
        id.to_string()
    } else {
        format!("minecraft:{id}")
    }
}
