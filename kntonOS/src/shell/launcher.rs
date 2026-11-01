// Deck Game Launcher and Quick-Access Hardware Menu.
// Provides title metadata catalog, layout grid, process broker and hardware controls.

pub const MAX_GAMES: usize = 32;

// Genre and classification for handheld game library catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameCategory {
    Action,
    Rpg,
    Strategy,
    Indie,
    Emulation,
    Tools,
}

// Metadata record describing an installed game title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameTitleMetadata {
    pub title: &'static str,
    pub executable_path: &'static str,
    pub developer: &'static str,
    pub category: GameCategory,
    pub rating: u8,
    pub launch_arguments: &'static str,
    pub play_time_seconds: u64,
}

impl GameTitleMetadata {
    // Construct a new game title metadata descriptor.
    pub const fn new(
        title: &'static str,
        executable_path: &'static str,
        developer: &'static str,
        category: GameCategory,
        rating: u8,
        launch_arguments: &'static str,
    ) -> Self {
        Self {
            title,
            executable_path,
            developer,
            category,
            rating,
            launch_arguments,
            play_time_seconds: 0,
        }
    }

    // Record additional play time in seconds.
    pub fn add_play_time(&mut self, seconds: u64) {
        self.play_time_seconds = self.play_time_seconds.saturating_add(seconds);
    }
}

// Catalog maintaining registered game titles and search indexing.
pub struct GameLibraryCatalog {
    entries: [Option<GameTitleMetadata>; MAX_GAMES],
    count: usize,
}

impl GameLibraryCatalog {
    // Initialize an empty game library catalog.
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_GAMES],
            count: 0,
        }
    }

    // Register a new game title metadata into the library.
    pub fn register_game(&mut self, metadata: GameTitleMetadata) -> Result<usize, &'static str> {
        if self.count >= MAX_GAMES {
            return Err("Catalog is at maximum capacity");
        }
        for (idx, slot) in self.entries.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(metadata);
                self.count += 1;
                return Ok(idx);
            }
        }
        Err("No free slot available")
    }

    // Retrieve game metadata by index.
    pub fn get_game(&self, index: usize) -> Option<&GameTitleMetadata> {
        if index < MAX_GAMES {
            self.entries[index].as_ref()
        } else {
            None
        }
    }

    // Retrieve mutable reference to game metadata by index.
    pub fn get_game_mut(&mut self, index: usize) -> Option<&mut GameTitleMetadata> {
        if index < MAX_GAMES {
            self.entries[index].as_mut()
        } else {
            None
        }
    }

    // Find first game matching the given title string.
    pub fn find_by_title(&self, title: &str) -> Option<(usize, &GameTitleMetadata)> {
        for (idx, slot) in self.entries.iter().enumerate() {
            if let Some(meta) = slot {
                if meta.title == title {
                    return Some((idx, meta));
                }
            }
        }
        None
    }

    // Count games in the catalog matching the specified category.
    pub fn count_by_category(&self, category: GameCategory) -> usize {
        let mut count = 0;
        for slot in self.entries.iter() {
            if let Some(meta) = slot {
                if meta.category == category {
                    count += 1;
                }
            }
        }
        count
    }

    // Return the total number of installed games in catalog.
    pub fn total_games(&self) -> usize {
        self.count
    }
}
