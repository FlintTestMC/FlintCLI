use crate::bot::TestBot;
use crate::executor::block;
use crate::executor::tick;
use anyhow::Result;
use flint_core::BlockPos;
use flint_core::test_spec::{Block, BlockFace, GameMode, Item, PlayerSlot};
use flint_core::traits::{FlintAdapter, FlintPlayer, FlintWorld, ServerInfo};

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
            focus: [0, 64, 0],
            current_tick: 0,
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
    pub focus: [i32; 3],
    pub current_tick: u64,
}

impl MinecraftWorld {
    fn world_pos(&self, pos: BlockPos) -> [i32; 3] {
        [
            pos[0] + self.offset[0],
            pos[1] + self.offset[1],
            pos[2] + self.offset[2],
        ]
    }

    pub fn ensure_focus(&self) -> Result<()> {
        self.bot.ensure_near(self.focus)
    }
}

impl Drop for MinecraftWorld {
    fn drop(&mut self) {
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
        let _ = self.bot.ensure_near(world_pos);
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
        let _ = self.bot.send_command(&cmd);
        std::thread::sleep(std::time::Duration::from_millis(tick::COMMAND_DELAY_MS));
    }

    fn create_player(&mut self) -> Box<dyn FlintPlayer> {
        Box::new(MinecraftPlayer {
            bot: self.bot.clone(),
            selected_hotbar: 1,
            inventory: std::collections::HashMap::new(),
            offset: self.offset,
            game_mode: GameMode::Creative,
        })
    }
}

pub struct MinecraftPlayer {
    bot: TestBot,
    selected_hotbar: u8,
    inventory: std::collections::HashMap<PlayerSlot, Item>,
    offset: [i32; 3],
    game_mode: GameMode,
}

pub fn slot_to_minecraft_name(slot: PlayerSlot) -> &'static str {
    match slot {
        PlayerSlot::Hotbar1 => "container.0",
        PlayerSlot::Hotbar2 => "container.1",
        PlayerSlot::Hotbar3 => "container.2",
        PlayerSlot::Hotbar4 => "container.3",
        PlayerSlot::Hotbar5 => "container.4",
        PlayerSlot::Hotbar6 => "container.5",
        PlayerSlot::Hotbar7 => "container.6",
        PlayerSlot::Hotbar8 => "container.7",
        PlayerSlot::Hotbar9 => "container.8",
        PlayerSlot::OffHand => "weapon.offhand",
        PlayerSlot::Helmet => "armor.head",
        PlayerSlot::Chestplate => "armor.chest",
        PlayerSlot::Leggings => "armor.legs",
        PlayerSlot::Boots => "armor.feet",
    }
}

impl FlintPlayer for MinecraftPlayer {
    fn set_slot(&mut self, slot: PlayerSlot, item: Option<&Item>) {
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
        std::thread::sleep(std::time::Duration::from_millis(tick::COMMAND_DELAY_MS));
    }

    fn get_slot(&self, slot: PlayerSlot, _requested_data: Vec<String>) -> Option<Item> {
        self.inventory.get(&slot).cloned()
    }

    fn select_hotbar(&mut self, slot: u8) {
        self.selected_hotbar = slot;
    }

    fn selected_hotbar(&self) -> u8 {
        self.selected_hotbar
    }

    fn use_item_on(&mut self, pos: BlockPos, face: &BlockFace) {
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
            _ => PlayerSlot::Hotbar1,
        };

        if let Some(item) = self.inventory.get(&slot) {
            let mut target_pos = pos;
            match face {
                BlockFace::Bottom => target_pos[1] -= 1,
                BlockFace::Top => target_pos[1] += 1,
                BlockFace::North => target_pos[2] -= 1,
                BlockFace::South => target_pos[2] += 1,
                BlockFace::West => target_pos[0] -= 1,
                BlockFace::East => target_pos[0] += 1,
            }

            let target_world = [
                target_pos[0] + self.offset[0],
                target_pos[1] + self.offset[1],
                target_pos[2] + self.offset[2],
            ];

            let _ = self.bot.ensure_near(target_world);

            let mut block_id = if item.id.contains("flint_and_steel") {
                "minecraft:fire".to_string()
            } else if item.id.contains(":") {
                item.id.clone()
            } else {
                format!("minecraft:{}", item.id)
            };

            if let Ok(Some(actual_block_str)) = self.bot.get_block(target_world)
                && actual_block_str.to_lowercase().contains("water")
            {
                let id_lower = block_id.to_lowercase();
                if id_lower.contains("pane")
                    || id_lower.contains("fence")
                    || id_lower.contains("wall")
                    || id_lower.contains("slab")
                    || id_lower.contains("stair")
                {
                    block_id = format!("{}[waterlogged=true]", block_id);
                }
            }

            let cmd = format!(
                "setblock {} {} {} {}",
                target_world[0], target_world[1], target_world[2], block_id
            );
            let _ = self.bot.send_command(&cmd);
            std::thread::sleep(std::time::Duration::from_millis(tick::COMMAND_DELAY_MS));

            if (self.game_mode == GameMode::Survival || self.game_mode == GameMode::Adventure)
                && !item.id.contains("flint_and_steel")
            {
                if item.count > 1 {
                    let mut updated_item = item.clone();
                    updated_item.count -= 1;
                    self.set_slot(slot, Some(&updated_item));
                } else {
                    self.set_slot(slot, None);
                }
            }
        }
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
