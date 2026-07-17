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
    fn create_test_world(&self) -> Result<Box<dyn FlintWorld>> {
        // Freeze time globally first when creating test world
        self.bot.send_command_synced("tick freeze")?;

        Ok(Box::new(MinecraftWorld {
            bot: self.bot.clone(),
            offset: [0, 0, 0],
            current_tick: 0,
            entities: HashMap::new(),
            entity_bounds: None,
        }))
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
    pub(crate) entity_bounds: Option<[[i32; 3]; 2]>,
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

    pub(crate) fn set_block_checked(&mut self, pos: BlockPos, block: &Block) -> Result<()> {
        let world_pos = self.world_pos(pos);
        let block_spec = block.to_command();
        let cmd = format!(
            "setblock {} {} {} {}",
            world_pos[0], world_pos[1], world_pos[2], block_spec
        );
        let expected = block.clone();
        self.bot.wait_for_block_chunk(world_pos)?;
        self.bot.send_command(&cmd)?;
        self.bot.sync_client_world()?;
        self.bot.wait_until("block synchronization", || {
            let Ok(Some(actual_block_str)) = self.bot.get_block(world_pos) else {
                return false;
            };
            let actual = block::make_block(&block::extract_block_id(&actual_block_str));
            actual.id == expected.id && block::properties_match(&actual, &expected)
        })
    }
}

impl Drop for MinecraftWorld {
    fn drop(&mut self) {
        for entity in self.entities.values() {
            let _ = self
                .bot
                .send_command(&format!("kill @e[tag={}]", entity.tag));
        }
        // Tick state is shared by the whole server, not owned by an individual
        // test world. The executor unfreezes once after the complete batch.
    }
}

impl FlintWorld for MinecraftWorld {
    fn do_tick(&mut self) -> Result<()> {
        let mut bot = self.bot.clone();
        tick::step_tick(&mut bot, false)?;
        self.current_tick += 1;
        Ok(())
    }

    fn current_tick(&self) -> u64 {
        self.current_tick
    }

    fn get_time(&self) -> Result<u64> {
        query_daytime(&self.bot)
    }

    fn get_block(&self, pos: BlockPos, requested_nbt: &[String]) -> Result<Block> {
        let world_pos = self.world_pos(pos);
        for _ in 0..10 {
            if let Ok(Some(actual_block_str)) = self.bot.get_block(world_pos) {
                let normalized_id = block::extract_block_id(&actual_block_str);
                let mut block = block::make_block(&normalized_id);
                if !normalized_id.is_empty() {
                    let mut values = Vec::with_capacity(requested_nbt.len());
                    for path in requested_nbt {
                        values.push((path.clone(), query_block_data(&self.bot, world_pos, path)?));
                    }
                    let nbt = flint_core::test_spec::EntityNbt::from_string_values(values);
                    if !requested_nbt.is_empty() {
                        block.nbt = Some(nbt);
                    }
                    return Ok(block);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        anyhow::bail!("timed out reading block at {world_pos:?}")
    }

    fn set_block(&mut self, pos: BlockPos, block: &Block) -> Result<()> {
        self.set_block_checked(pos, block)
    }

    fn summon_entity(
        &mut self,
        alias: &str,
        entity_type: &str,
        pos: [f64; 3],
        nbt: Option<&EntityNbt>,
    ) -> Result<()> {
        let Some(tag) = Self::entity_tag(alias) else {
            anyhow::bail!("invalid entity alias for summon: {alias}");
        };
        if entity_type
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == ':' || c == '.'))
        {
            anyhow::bail!("invalid entity type for summon: {entity_type}");
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
        self.bot.send_command(&cmd)?;
        self.entities.insert(
            alias.to_string(),
            MinecraftEntity {
                entity_type: entity_type.to_string(),
                tag,
            },
        );
        Ok(())
    }

    fn teleport_entity(&mut self, alias: &str, pos: [f64; 3], rot: Option<[f32; 2]>) -> Result<()> {
        let Some(entity) = self.entities.get(alias) else {
            anyhow::bail!("cannot teleport unknown entity alias: {alias}");
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
        self.bot.send_command(&cmd)
    }

    fn get_entity(&self, alias: &str, requested_nbt: &[String]) -> Result<Vec<EntityState>> {
        let Some(entity) = self.entities.get(alias) else {
            return Ok(Vec::new());
        };
        let selector = Self::entity_selector(entity);
        let values = query_entity_numbers(&self.bot, &selector, "Pos")?;
        let pos = values.get(..3).map(|pos| {
            [
                pos[0] - f64::from(self.offset[0]),
                pos[1] - f64::from(self.offset[1]),
                pos[2] - f64::from(self.offset[2]),
            ]
        });
        let rotation = query_entity_numbers(&self.bot, &selector, "Rotation")?;
        let rot = rotation.get(..2).map(|rot| [rot[0] as f32, rot[1] as f32]);
        let mut nbt = HashMap::new();
        for path in requested_nbt {
            nbt.insert(path.clone(), query_entity_data(&self.bot, &selector, path)?);
        }
        Ok(vec![EntityState {
            entity_type: Some(entity.entity_type.clone()),
            pos,
            rot,
            nbt,
        }])
    }

    fn find_entity(&self, entity_type: &str, requested_nbt: &[String]) -> Result<Vec<EntityState>> {
        if entity_type
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == ':' || c == '.'))
        {
            anyhow::bail!("invalid entity type for lookup: {entity_type}");
        }
        let all_selector = if let Some([min, max]) = self.entity_bounds {
            format!(
                "@e[type={entity_type},x={},y={},z={},dx={},dy={},dz={}]",
                min[0],
                min[1],
                min[2],
                max[0] - min[0],
                max[1] - min[1],
                max[2] - min[2]
            )
        } else {
            format!("@e[type={entity_type}]")
        };
        let count = query_entity_count(&self.bot, &all_selector)?;
        if count == 0 {
            return Ok(Vec::new());
        }

        const SCANNED_TAG: &str = "flintmc.assert.scanned";
        self.bot
            .send_command_synced(&format!("tag {all_selector} remove {SCANNED_TAG}"))?;
        let mut entities = Vec::with_capacity(count);
        for _ in 0..count {
            let selector = all_selector.replacen(
                "@e[",
                &format!("@e[tag=!{SCANNED_TAG},sort=nearest,limit=1,"),
                1,
            );
            let values = query_entity_numbers(&self.bot, &selector, "Pos")?;
            let pos = values.get(..3).map(|pos| {
                [
                    pos[0] - f64::from(self.offset[0]),
                    pos[1] - f64::from(self.offset[1]),
                    pos[2] - f64::from(self.offset[2]),
                ]
            });
            let rotation = query_entity_numbers(&self.bot, &selector, "Rotation")?;
            let rot = rotation.get(..2).map(|rot| [rot[0] as f32, rot[1] as f32]);
            let mut nbt = HashMap::new();
            for path in requested_nbt {
                nbt.insert(path.clone(), query_entity_data(&self.bot, &selector, path)?);
            }
            entities.push(EntityState {
                entity_type: Some(entity_type.to_string()),
                pos,
                rot,
                nbt,
            });
            self.bot
                .send_command_synced(&format!("tag {selector} add {SCANNED_TAG}"))?;
        }
        self.bot
            .send_command_synced(&format!("tag {all_selector} remove {SCANNED_TAG}"))?;
        Ok(entities)
    }

    fn create_player(&mut self) -> Box<dyn FlintPlayer> {
        Box::new(MinecraftPlayer {
            bot: self.bot.clone(),
            inventory_owner: self.bot.allocate_inventory_owner(),
            selected_hotbar: 1,
            inventory: std::collections::HashMap::new(),
            position: None,
            rotation: None,
            offset: self.offset,
            game_mode: GameMode::Creative,
        })
    }

    fn fill(&mut self, region: [[i32; 3]; 2], block: &Block) -> Result<()> {
        let world_min = self.world_pos(region[0]);
        let world_max = self.world_pos(region[1]);
        self.bot.send_command_synced(&format!(
            "fill {} {} {} {} {} {} {}",
            world_min[0],
            world_min[1],
            world_min[2],
            world_max[0],
            world_max[1],
            world_max[2],
            block.to_command()
        ))
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

fn query_entity_count(bot: &TestBot, selector: &str) -> Result<usize> {
    let _query_guard = bot.lock_command_query();
    drain_chat(bot);
    bot.send_command(&format!("execute if entity {selector}"))?;
    let timeout = std::time::Duration::from_secs(3);
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        if let Some((sender, message)) =
            bot.recv_chat_timeout(std::time::Duration::from_millis(tick::CHAT_POLL_TIMEOUT_MS))
        {
            if sender.is_some() {
                continue;
            }
            if message.contains("Test failed") || message.contains("No entity was found") {
                return Ok(0);
            }
            if let Some(count) = message
                .split(|character: char| !character.is_ascii_digit())
                .rfind(|part| !part.is_empty())
                .and_then(|part| part.parse::<usize>().ok())
            {
                return Ok(count);
            }
        }
    }
    anyhow::bail!("timed out counting entities matching {selector}")
}

pub(crate) fn query_daytime(bot: &TestBot) -> Result<u64> {
    query_time_command(bot, "time query minecraft:day", "daytime").map(|time| time % 24_000)
}

pub(crate) fn query_gametime(bot: &TestBot) -> Result<u64> {
    query_time_command(bot, "time query gametime", "game time")
}

fn query_time_command(bot: &TestBot, command: &str, label: &str) -> Result<u64> {
    let _query_guard = bot.lock_command_query();
    drain_chat(bot);

    bot.send_command(command)?;
    let timeout = std::time::Duration::from_secs(3);
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        if let Some((sender, message)) =
            bot.recv_chat_timeout(std::time::Duration::from_millis(tick::CHAT_POLL_TIMEOUT_MS))
        {
            if sender.is_some() || !message.to_ascii_lowercase().contains("time") {
                continue;
            }
            if let Some(time) = message
                .split(|character: char| !character.is_ascii_digit())
                .rfind(|part| !part.is_empty())
                .and_then(|part| part.parse::<u64>().ok())
            {
                return Ok(time);
            }
            anyhow::bail!("time query returned no numeric {label}: {message}");
        }
    }
    anyhow::bail!("timed out querying world {label}")
}

fn query_entity_data(bot: &TestBot, selector: &str, path: &str) -> Result<String> {
    let message = query_entity_data_message(bot, selector, path)?;
    Ok(message
        .split_once(':')
        .map(|(_, value)| value.trim().to_string())
        .unwrap_or_else(|| message.trim().to_string()))
}

fn query_block_data(bot: &TestBot, pos: BlockPos, path: &str) -> Result<String> {
    let _query_guard = bot.lock_command_query();
    drain_chat(bot);
    bot.send_command(&format!(
        "data get block {} {} {} {path}",
        pos[0], pos[1], pos[2]
    ))?;
    let timeout = std::time::Duration::from_secs(3);
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        if let Some((sender, message)) =
            bot.recv_chat_timeout(std::time::Duration::from_millis(tick::CHAT_POLL_TIMEOUT_MS))
        {
            if sender.is_some() {
                continue;
            }
            if message.contains("is not a block entity")
                || message.contains("Found no elements")
                || message.contains("No elements matching")
            {
                anyhow::bail!("block entity query failed: {message}");
            }
            if message.contains(path) || message.contains("block data") {
                return Ok(message
                    .split_once(':')
                    .map(|(_, value)| value.trim().to_string())
                    .unwrap_or_else(|| message.trim().to_string()));
            }
        }
    }
    anyhow::bail!(
        "timed out querying block entity at [{}, {}, {}] {path}",
        pos[0],
        pos[1],
        pos[2]
    )
}

fn query_entity_data_message(bot: &TestBot, selector: &str, path: &str) -> Result<String> {
    let _query_guard = bot.lock_command_query();
    drain_chat(bot);

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

fn drain_chat(bot: &TestBot) {
    while bot
        .recv_chat_timeout(std::time::Duration::from_millis(
            tick::CHAT_DRAIN_TIMEOUT_MS,
        ))
        .is_some()
    {}
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
    position: Option<[f64; 3]>,
    rotation: Option<[f32; 2]>,
    offset: [i32; 3],
    game_mode: GameMode,
}

impl FlintPlayer for MinecraftPlayer {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn set_slot(&mut self, slot: PlayerSlot, item: Option<&Item>) -> Result<()> {
        self.set_slot_checked(slot, item)
    }

    fn get_slot(&mut self, slot: PlayerSlot, _requested_data: Vec<String>) -> Result<Option<Item>> {
        self.restore_inventory()?;
        Ok(self.inventory.get(&slot).cloned())
    }

    fn select_hotbar(&mut self, slot: u8) -> Result<()> {
        self.select_hotbar_checked(slot)
    }

    fn selected_hotbar(&self) -> u8 {
        self.selected_hotbar
    }

    fn teleport(&mut self, pos: [f64; 3], rot: Option<[f32; 2]>) -> Result<()> {
        self.teleport_checked(pos, rot)
    }

    fn interact(&mut self) -> Result<()> {
        self.interact_checked()
    }

    fn set_game_mode(&mut self, mode: GameMode) -> Result<()> {
        self.set_game_mode_checked(mode)
    }
}

impl MinecraftPlayer {
    pub(crate) fn set_slot_checked(&mut self, slot: PlayerSlot, item: Option<&Item>) -> Result<()> {
        self.restore_state_checked()?;
        if let Some(item) = item {
            self.inventory.insert(slot, item.clone());
        } else {
            self.inventory.remove(&slot);
        }
        let slot_name = slot_to_minecraft_name(slot);
        let command = match item {
            Some(item) => format!(
                "item replace entity flintmc_testbot {} with {} {}",
                slot_name, item.id, item.count
            ),
            None => format!("item replace entity flintmc_testbot {} with air", slot_name),
        };
        self.bot.send_command_synced(&command)?;
        self.bot.wait_for_inventory(&self.inventory)?;
        self.record_state();
        Ok(())
    }

    pub(crate) fn select_hotbar_checked(&mut self, slot: u8) -> Result<()> {
        self.restore_state_checked()?;
        self.bot.select_hotbar(slot)?;
        self.selected_hotbar = slot;
        self.record_state();
        Ok(())
    }

    pub(crate) fn teleport_checked(
        &mut self,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> Result<()> {
        self.restore_state_checked()?;
        let world_pos = [
            pos[0] + f64::from(self.offset[0]),
            pos[1] + f64::from(self.offset[1]),
            pos[2] + f64::from(self.offset[2]),
        ];
        self.bot.teleport(world_pos, rotation)?;
        self.position = Some(world_pos);
        self.rotation = rotation;
        self.record_state();
        Ok(())
    }

    pub(crate) fn interact_checked(&mut self) -> Result<()> {
        self.restore_state_checked()?;
        self.bot.interact()?;
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
                _ => anyhow::bail!("invalid selected hotbar slot: {}", self.selected_hotbar),
            };
            let observed = self.bot.inventory_slot(slot)?;
            reconcile_inventory_slot(&mut self.inventory, slot, observed);
        }
        self.bot.wait_for_inventory(&self.inventory)?;
        self.record_state();
        Ok(())
    }

    pub(crate) fn set_game_mode_checked(&mut self, mode: GameMode) -> Result<()> {
        self.restore_state_checked()?;
        let mode_name = match mode {
            GameMode::Survival => "survival",
            GameMode::Creative => "creative",
            GameMode::Adventure => "adventure",
            GameMode::Spectator => "spectator",
        };
        self.bot
            .send_command_synced(&format!("gamemode {mode_name} flintmc_testbot"))?;
        self.game_mode = mode;
        self.record_state();
        Ok(())
    }

    pub(crate) fn restore_inventory(&mut self) -> Result<()> {
        self.restore_state_checked()
    }

    fn restore_state_checked(&mut self) -> Result<()> {
        self.bot.restore_player(
            self.inventory_owner,
            &self.inventory,
            self.selected_hotbar,
            self.position,
            self.rotation,
            self.game_mode,
        )
    }

    fn record_state(&self) {
        self.bot.record_player(
            self.inventory_owner,
            &self.inventory,
            self.selected_hotbar,
            self.position,
            self.rotation,
            self.game_mode,
        );
    }
}

fn reconcile_inventory_slot(
    inventory: &mut std::collections::HashMap<PlayerSlot, Item>,
    slot: PlayerSlot,
    observed: Option<Item>,
) {
    match observed {
        Some(item) => {
            inventory.insert(slot, item);
        }
        None => {
            inventory.remove(&slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::reconcile_inventory_slot;
    use flint_core::test_spec::{Item, PlayerSlot};
    use std::collections::HashMap;

    #[test]
    fn interaction_does_not_consume_item_when_server_reports_unchanged_stack() {
        let slot = PlayerSlot::Hotbar1;
        let mut inventory = HashMap::from([(slot, Item::with_count("minecraft:stick", 2))]);

        reconcile_inventory_slot(
            &mut inventory,
            slot,
            Some(Item::with_count("minecraft:stick", 2)),
        );

        assert_eq!(
            inventory.get(&slot),
            Some(&Item::with_count("minecraft:stick", 2))
        );
    }

    #[test]
    fn interaction_uses_consumed_stack_reported_by_server() {
        let slot = PlayerSlot::Hotbar1;
        let mut inventory = HashMap::from([(slot, Item::with_count("minecraft:oak_sign", 2))]);

        reconcile_inventory_slot(
            &mut inventory,
            slot,
            Some(Item::with_count("minecraft:oak_sign", 1)),
        );

        assert_eq!(
            inventory.get(&slot),
            Some(&Item::with_count("minecraft:oak_sign", 1))
        );
    }
}
