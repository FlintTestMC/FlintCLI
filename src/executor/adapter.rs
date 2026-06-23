use crate::bot::TestBot;
use crate::executor::block;
use crate::executor::tick;
use flint_core::traits::{FlintAdapter, FlintWorld, FlintPlayer, ServerInfo};
use flint_core::test_spec::{Block, Item, PlayerSlot, BlockFace, GameMode};
use flint_core::BlockPos;

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
        let world_pos = [pos[0] + self.offset[0], pos[1] + self.offset[1], pos[2] + self.offset[2]];
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
        let world_pos = [pos[0] + self.offset[0], pos[1] + self.offset[1], pos[2] + self.offset[2]];
        let block_spec = block.to_command();
        let cmd = format!("setblock {} {} {} {}", world_pos[0], world_pos[1], world_pos[2], block_spec);
        let _ = self.bot.send_command(&cmd);
        std::thread::sleep(std::time::Duration::from_millis(tick::COMMAND_DELAY_MS));
    }

    fn create_player(&mut self) -> Box<dyn FlintPlayer> {
        Box::new(MinecraftPlayer {
            bot: self.bot.clone(),
        })
    }
}

pub struct MinecraftPlayer {
    bot: TestBot,
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
            format!("item replace entity flintmc_testbot {} with {} {}", slot_name, it.id, it.count)
        } else {
            format!("item replace entity flintmc_testbot {} with air", slot_name)
        };
        let _ = self.bot.send_command(&cmd);
        std::thread::sleep(std::time::Duration::from_millis(tick::COMMAND_DELAY_MS));
    }

    fn get_slot(&self, _slot: PlayerSlot, _requested_data: Vec<String>) -> Option<Item> {
        None
    }

    fn select_hotbar(&mut self, _slot: u8) {
    }

    fn selected_hotbar(&self) -> u8 {
        1
    }

    fn use_item_on(&mut self, _pos: BlockPos, _face: &BlockFace) {
    }

    fn set_game_mode(&mut self, mode: GameMode) {
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
