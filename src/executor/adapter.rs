use crate::bot::{TestBot, slot_to_minecraft_name};
use crate::executor::block;
use crate::executor::tick;
use anyhow::Result;
use flint_core::BlockPos;
use flint_core::test_spec::{Block, EntityNbt, GameMode, Item, PlayerSlot};
use flint_core::traits::{EntityState, FlintAdapter, FlintPlayer, FlintWorld, ServerInfo};
use std::collections::HashMap;

#[allow(dead_code)]
pub struct MinecraftAdapter {
    bot: TestBot,
}

#[allow(dead_code)]
impl MinecraftAdapter {
    pub fn new(bot: TestBot) -> Self {
        Self { bot }
    }
}

impl FlintAdapter for MinecraftAdapter {
    fn create_test_world(&self) -> Box<dyn FlintWorld> {
        // Freeze time globally first when creating test world
        let _ = self.bot.send_command("tick freeze");
        std::thread::sleep(std::time::Duration::from_millis(tick::COMMAND_DELAY_MS));

        Box::new(MinecraftWorld {
            bot: self.bot.clone(),
            offset: [0, 0, 0],
            current_tick: 0,
            entities: HashMap::new(),
        })
    }

    fn server_info(&self) -> ServerInfo {
        ServerInfo {
            minecraft_version: "1.21.8".to_string(),
        }
    }
}

pub struct MinecraftWorld {
    pub bot: TestBot,
    pub offset: [i32; 3],
    pub current_tick: u64,
    pub(crate) entities: HashMap<String, MinecraftEntity>,
}

#[derive(Debug, Clone)]
pub(crate) struct MinecraftEntity {
    entity_type: String,
    tag: String,
}

impl MinecraftWorld {
    fn world_pos(&self, pos: BlockPos) -> [i32; 3] {
        [
            pos[0] + self.offset[0],
            pos[1] + self.offset[1],
            pos[2] + self.offset[2],
        ]
    }

    fn entity_tag(alias: &str) -> Option<String> {
        if alias == "player"
            || alias.is_empty()
            || alias
                .chars()
                .any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'))
        {
            return None;
        }
        Some(format!("flintmc.entity.{alias}"))
    }

    fn entity_selector(entity: &MinecraftEntity) -> String {
        format!("@e[tag={},type={},limit=1]", entity.tag, entity.entity_type)
    }
}

impl Drop for MinecraftWorld {
    fn drop(&mut self) {
        for entity in self.entities.values() {
            let _ = self
                .bot
                .send_command(&format!("kill @e[tag={}]", entity.tag));
        }
        let _ = self.bot.send_command("tick unfreeze");
        std::thread::sleep(std::time::Duration::from_millis(tick::COMMAND_DELAY_MS));
    }
}

impl FlintWorld for MinecraftWorld {
    fn do_tick(&mut self) {
        let mut bot = self.bot.clone();
        let _ = tick::step_tick(&mut bot, false);
        self.current_tick += 1;
    }

    fn current_tick(&self) -> u64 {
        self.current_tick
    }

    fn get_block(&self, pos: BlockPos) -> Block {
        let world_pos = self.world_pos(pos);
        for _ in 0..10 {
            if let Ok(Some(actual_block_str)) = self.bot.get_block(world_pos) {
                let normalized_id = block::extract_block_id(&actual_block_str);
                let block = block::make_block(&normalized_id);
                if !normalized_id.is_empty() {
                    return block;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        Block {
            id: "minecraft:air".to_string(),
            properties: Default::default(),
        }
    }

    fn set_block(&mut self, pos: BlockPos, block: &Block) {
        let world_pos = self.world_pos(pos);
        let block_spec = block.to_command();
        let cmd = format!(
            "setblock {} {} {} {}",
            world_pos[0], world_pos[1], world_pos[2], block_spec
        );
        let expected = block.clone();
        if let Err(error) = self.bot.send_command(&cmd).and_then(|()| {
            self.bot.wait_until("block synchronization", || {
                let Ok(Some(actual_block_str)) = self.bot.get_block(world_pos) else {
                    return false;
                };
                let actual = block::make_block(&block::extract_block_id(&actual_block_str));
                actual.id == expected.id && block::properties_match(&actual, &expected)
            })
        }) {
            tracing::error!(
                "Failed to set block at [{}, {}, {}] to {}: {}",
                world_pos[0],
                world_pos[1],
                world_pos[2],
                block_spec,
                error
            );
        }
    }

    fn summon_entity(
        &mut self,
        alias: &str,
        entity_type: &str,
        pos: [f64; 3],
        nbt: Option<&EntityNbt>,
    ) {
        let Some(tag) = Self::entity_tag(alias) else {
            tracing::error!("Invalid entity alias for summon: {}", alias);
            return;
        };
        if entity_type
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == ':' || c == '.'))
        {
            tracing::error!("Invalid entity type for summon: {}", entity_type);
            return;
        }
        let world_pos = [
            pos[0] + f64::from(self.offset[0]),
            pos[1] + f64::from(self.offset[1]),
            pos[2] + f64::from(self.offset[2]),
        ];
        let _ = self.bot.send_command(&format!("kill @e[tag={}]", tag));
        let nbt = summon_nbt_with_tag(nbt.map(EntityNbt::to_snbt).as_deref(), &tag);
        let cmd = format!(
            "summon {} {} {} {} {}",
            entity_type, world_pos[0], world_pos[1], world_pos[2], nbt
        );
        if let Err(error) = self.bot.send_command(&cmd) {
            tracing::error!("Failed to summon entity alias {}: {}", alias, error);
            return;
        }
        self.entities.insert(
            alias.to_string(),
            MinecraftEntity {
                entity_type: entity_type.to_string(),
                tag,
            },
        );
    }

    fn teleport_entity(&mut self, alias: &str, pos: [f64; 3], rot: Option<[f32; 2]>) {
        let Some(entity) = self.entities.get(alias) else {
            tracing::error!("Cannot teleport unknown entity alias: {}", alias);
            return;
        };
        let selector = Self::entity_selector(entity);
        let world_pos = [
            pos[0] + f64::from(self.offset[0]),
            pos[1] + f64::from(self.offset[1]),
            pos[2] + f64::from(self.offset[2]),
        ];
        let cmd = match rot {
            Some([yaw, pitch]) => format!(
                "tp {} {} {} {} {} {}",
                selector, world_pos[0], world_pos[1], world_pos[2], yaw, pitch
            ),
            None => format!(
                "tp {} {} {} {}",
                selector, world_pos[0], world_pos[1], world_pos[2]
            ),
        };
        if let Err(error) = self.bot.send_command(&cmd) {
            tracing::error!("Failed to teleport entity alias {}: {}", alias, error);
        }
    }

    fn get_entity(&self, alias: &str, requested_nbt: &[String]) -> EntityState {
        let Some(entity) = self.entities.get(alias) else {
            return EntityState::default();
        };
        let selector = Self::entity_selector(entity);
        let Ok(values) = query_entity_numbers(&self.bot, &selector, "Pos") else {
            return EntityState {
                exists: false,
                entity_type: Some(entity.entity_type.clone()),
                pos: None,
                rot: None,
                nbt: HashMap::new(),
            };
        };
        let pos = values.get(..3).map(|pos| {
            [
                pos[0] - f64::from(self.offset[0]),
                pos[1] - f64::from(self.offset[1]),
                pos[2] - f64::from(self.offset[2]),
            ]
        });
        let rot = query_entity_numbers(&self.bot, &selector, "Rotation")
            .ok()
            .and_then(|rot| rot.get(..2).map(|rot| [rot[0] as f32, rot[1] as f32]));
        let mut nbt = HashMap::new();
        for path in requested_nbt {
            if let Ok(value) = query_entity_data(&self.bot, &selector, path) {
                nbt.insert(path.clone(), value);
            }
        }
        EntityState {
            exists: true,
            entity_type: Some(entity.entity_type.clone()),
            pos,
            rot,
            nbt,
        }
    }

    fn create_player(&mut self) -> Box<dyn FlintPlayer> {
        Box::new(MinecraftPlayer {
            bot: self.bot.clone(),
            inventory_owner: self.bot.allocate_inventory_owner(),
            selected_hotbar: 1,
            inventory: std::collections::HashMap::new(),
            pose: None,
            offset: self.offset,
            game_mode: GameMode::Creative,
        })
    }
}

fn query_entity_numbers(bot: &TestBot, selector: &str, path: &str) -> Result<Vec<f64>> {
    let message = query_entity_data_message(bot, selector, path)?;
    let values = parse_numbers_after_colon(&message);
    if values.is_empty() {
        anyhow::bail!("entity query returned no numbers: {message}");
    }
    Ok(values)
}

fn query_entity_data(bot: &TestBot, selector: &str, path: &str) -> Result<String> {
    let message = query_entity_data_message(bot, selector, path)?;
    Ok(message
        .split_once(':')
        .map(|(_, value)| value.trim().to_string())
        .unwrap_or_else(|| message.trim().to_string()))
}

fn query_entity_data_message(bot: &TestBot, selector: &str, path: &str) -> Result<String> {
    while bot
        .recv_chat_timeout(std::time::Duration::from_millis(
            tick::CHAT_DRAIN_TIMEOUT_MS,
        ))
        .is_some()
    {}

    bot.send_command(&format!("data get entity {selector} {path}"))?;
    let timeout = std::time::Duration::from_secs(3);
    let started = std::time::Instant::now();

    while started.elapsed() < timeout {
        if let Some((_, message)) =
            bot.recv_chat_timeout(std::time::Duration::from_millis(tick::CHAT_POLL_TIMEOUT_MS))
        {
            if message.contains("No entity was found") || message.contains("Found no elements") {
                anyhow::bail!("entity query failed: {message}");
            }
            if message.contains(path) || message.contains("entity data") {
                return Ok(message);
            }
        }
    }

    anyhow::bail!("timed out querying entity {selector} {path}")
}

fn parse_numbers_after_colon(message: &str) -> Vec<f64> {
    let value_part = message
        .split_once(':')
        .map(|(_, value)| value)
        .unwrap_or(message);
    value_part
        .split(|c: char| {
            !(c.is_ascii_digit() || c == '-' || c == '+' || c == '.' || c == 'e' || c == 'E')
        })
        .filter_map(|part| {
            if part.is_empty() || part == "-" || part == "+" || part == "." {
                None
            } else {
                part.parse::<f64>().ok()
            }
        })
        .collect()
}

fn summon_nbt_with_tag(nbt: Option<&str>, tag: &str) -> String {
    match nbt.map(str::trim).filter(|nbt| !nbt.is_empty()) {
        None => format!("{{Tags:[\"{tag}\"]}}"),
        Some("{}") => format!("{{Tags:[\"{tag}\"]}}"),
        Some(nbt) if nbt.starts_with('{') && nbt.ends_with('}') => {
            let inner = &nbt[1..nbt.len() - 1];
            if inner.trim().is_empty() {
                format!("{{Tags:[\"{tag}\"]}}")
            } else {
                format!("{{{inner},Tags:[\"{tag}\"]}}")
            }
        }
        Some(nbt) => {
            tracing::warn!("Invalid summon NBT '{}', using only FlintMC alias tag", nbt);
            format!("{{Tags:[\"{tag}\"]}}")
        }
    }
}

pub struct MinecraftPlayer {
    bot: TestBot,
    inventory_owner: u64,
    selected_hotbar: u8,
    inventory: std::collections::HashMap<PlayerSlot, Item>,
    pose: Option<([f64; 3], Option<[f32; 2]>)>,
    offset: [i32; 3],
    game_mode: GameMode,
}

impl FlintPlayer for MinecraftPlayer {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn set_slot(&mut self, slot: PlayerSlot, item: Option<&Item>) {
        self.restore_inventory();
        let slot_name = slot_to_minecraft_name(slot);
        let cmd = if let Some(it) = item {
            self.inventory.insert(slot, it.clone());
            format!(
                "item replace entity flintmc_testbot {} with {} {}",
                slot_name, it.id, it.count
            )
        } else {
            self.inventory.remove(&slot);
            format!("item replace entity flintmc_testbot {} with air", slot_name)
        };
        let _ = self.bot.send_command(&cmd);
        let _ = self.bot.wait_for_inventory(&self.inventory);
        self.bot
            .record_inventory(self.inventory_owner, &self.inventory, self.selected_hotbar);
    }

    fn get_slot(&self, slot: PlayerSlot, _requested_data: Vec<String>) -> Option<Item> {
        self.inventory.get(&slot).cloned()
    }

    fn select_hotbar(&mut self, slot: u8) {
        self.restore_inventory();
        self.selected_hotbar = slot;
        let _ = self.bot.select_hotbar(slot);
        self.bot
            .record_inventory(self.inventory_owner, &self.inventory, self.selected_hotbar);
    }

    fn selected_hotbar(&self) -> u8 {
        self.selected_hotbar
    }

    fn teleport(&mut self, pos: [f64; 3], rot: Option<[f32; 2]>) {
        let world_pos = [
            pos[0] + f64::from(self.offset[0]),
            pos[1] + f64::from(self.offset[1]),
            pos[2] + f64::from(self.offset[2]),
        ];
        self.pose = Some((world_pos, rot));
        let _ = self.bot.teleport(world_pos, rot);
    }

    fn interact(&mut self) {
        let mode_str = match self.game_mode {
            GameMode::Survival => "survival",
            GameMode::Creative => "creative",
            GameMode::Adventure => "adventure",
            GameMode::Spectator => "spectator",
        };
        let _ = self
            .bot
            .send_command(&format!("gamemode {} flintmc_testbot", mode_str));
        std::thread::sleep(std::time::Duration::from_millis(tick::COMMAND_DELAY_MS));
        self.restore_inventory();
        let _ = self.bot.keep_airborne();
        if let Some((pos, rot)) = self.pose {
            let _ = self.bot.teleport(pos, rot);
        }
        let _ = self.bot.interact();

        if self.game_mode == GameMode::Survival || self.game_mode == GameMode::Adventure {
            let slot = match self.selected_hotbar {
                1 => PlayerSlot::Hotbar1,
                2 => PlayerSlot::Hotbar2,
                3 => PlayerSlot::Hotbar3,
                4 => PlayerSlot::Hotbar4,
                5 => PlayerSlot::Hotbar5,
                6 => PlayerSlot::Hotbar6,
                7 => PlayerSlot::Hotbar7,
                8 => PlayerSlot::Hotbar8,
                9 => PlayerSlot::Hotbar9,
                _ => return,
            };
            if let Some(item) = self.inventory.get_mut(&slot) {
                if item.count > 1 {
                    item.count -= 1;
                } else {
                    self.inventory.remove(&slot);
                }
            }
        }
        let _ = self.bot.wait_for_inventory(&self.inventory);
        self.bot
            .record_inventory(self.inventory_owner, &self.inventory, self.selected_hotbar);
    }

    fn set_game_mode(&mut self, mode: GameMode) {
        self.game_mode = mode;
        let mode_str = match mode {
            GameMode::Survival => "survival",
            GameMode::Creative => "creative",
            GameMode::Adventure => "adventure",
            GameMode::Spectator => "spectator",
        };
        let cmd = format!("gamemode {} flintmc_testbot", mode_str);
        let _ = self.bot.send_command(&cmd);
        std::thread::sleep(std::time::Duration::from_millis(tick::COMMAND_DELAY_MS));
    }
}

impl MinecraftPlayer {
    pub(crate) fn restore_inventory(&mut self) {
        let _ =
            self.bot
                .restore_inventory(self.inventory_owner, &self.inventory, self.selected_hotbar);
    }
}
