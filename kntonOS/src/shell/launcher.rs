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

// Thumbnail grid layout engine with controller navigation and auto-scrolling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThumbnailGrid {
    pub columns: usize,
    pub rows_visible: usize,
    pub cell_width: u32,
    pub cell_height: u32,
    pub padding_x: u32,
    pub padding_y: u32,
    pub selected_cell: usize,
    pub scrolling_offset_row: usize,
    pub total_items: usize,
}

impl ThumbnailGrid {
    // Construct thumbnail grid layout with specified dimension and paging parameters.
    pub const fn new(
        columns: usize,
        rows_visible: usize,
        cell_width: u32,
        cell_height: u32,
        padding_x: u32,
        padding_y: u32,
    ) -> Self {
        Self {
            columns: if columns == 0 { 1 } else { columns },
            rows_visible: if rows_visible == 0 { 1 } else { rows_visible },
            cell_width,
            cell_height,
            padding_x,
            padding_y,
            selected_cell: 0,
            scrolling_offset_row: 0,
            total_items: 0,
        }
    }

    // Update total count of navigable items and clamp current selection.
    pub fn set_total_items(&mut self, count: usize) {
        self.total_items = count;
        if count == 0 {
            self.selected_cell = 0;
            self.scrolling_offset_row = 0;
        } else if self.selected_cell >= count {
            self.selected_cell = count - 1;
            self.adjust_scroll_to_selected();
        }
    }

    // Ensure scrolling offset keeps selected cell in visible rows window.
    pub fn adjust_scroll_to_selected(&mut self) {
        let current_row = self.selected_cell / self.columns;
        if current_row < self.scrolling_offset_row {
            self.scrolling_offset_row = current_row;
        } else if current_row >= self.scrolling_offset_row + self.rows_visible {
            self.scrolling_offset_row = current_row + 1 - self.rows_visible;
        }
    }

    // Navigate selection left within grid bounds.
    pub fn move_left(&mut self) -> bool {
        if self.total_items == 0 || self.selected_cell == 0 {
            return false;
        }
        self.selected_cell -= 1;
        self.adjust_scroll_to_selected();
        true
    }

    // Navigate selection right within grid bounds.
    pub fn move_right(&mut self) -> bool {
        if self.total_items == 0 || self.selected_cell + 1 >= self.total_items {
            return false;
        }
        self.selected_cell += 1;
        self.adjust_scroll_to_selected();
        true
    }

    // Navigate selection up by one row.
    pub fn move_up(&mut self) -> bool {
        if self.selected_cell >= self.columns {
            self.selected_cell -= self.columns;
            self.adjust_scroll_to_selected();
            true
        } else {
            false
        }
    }

    // Navigate selection down by one row.
    pub fn move_down(&mut self) -> bool {
        if self.selected_cell + self.columns < self.total_items {
            self.selected_cell += self.columns;
            self.adjust_scroll_to_selected();
            true
        } else {
            false
        }
    }

    // Scroll visible page up by rows_visible rows.
    pub fn page_up(&mut self) -> bool {
        let step = self.columns * self.rows_visible;
        if self.selected_cell >= step {
            self.selected_cell -= step;
        } else {
            self.selected_cell = 0;
        }
        self.adjust_scroll_to_selected();
        true
    }

    // Scroll visible page down by rows_visible rows.
    pub fn page_down(&mut self) -> bool {
        if self.total_items == 0 {
            return false;
        }
        let step = self.columns * self.rows_visible;
        if self.selected_cell + step < self.total_items {
            self.selected_cell += step;
        } else {
            self.selected_cell = self.total_items - 1;
        }
        self.adjust_scroll_to_selected();
        true
    }

    // Directly select a cell index if within range.
    pub fn select_cell(&mut self, index: usize) -> bool {
        if index < self.total_items {
            self.selected_cell = index;
            self.adjust_scroll_to_selected();
            true
        } else {
            false
        }
    }

    // Calculate layout coordinates (x, y, w, h) for a cell relative to grid viewport.
    pub fn cell_rect(&self, index: usize, origin_x: u32, origin_y: u32) -> Option<(u32, u32, u32, u32)> {
        if index >= self.total_items {
            return None;
        }
        let col = (index % self.columns) as u32;
        let row = index / self.columns;
        if row < self.scrolling_offset_row || row >= self.scrolling_offset_row + self.rows_visible {
            return None;
        }
        let visible_row = (row - self.scrolling_offset_row) as u32;
        let x = origin_x + col * (self.cell_width + self.padding_x);
        let y = origin_y + visible_row * (self.cell_height + self.padding_y);
        Some((x, y, self.cell_width, self.cell_height))
    }

    // Check if cell is currently in the visible viewport.
    pub fn is_cell_visible(&self, index: usize) -> bool {
        if index >= self.total_items {
            return false;
        }
        let row = index / self.columns;
        row >= self.scrolling_offset_row && row < self.scrolling_offset_row + self.rows_visible
    }
}

// Lifecycle state of the managed game process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchState {
    Idle,
    Preparing { title_index: usize },
    Running { pid: u32, title_index: usize, start_tick: u64 },
    Suspended { pid: u32, title_index: usize },
    Terminated { pid: u32, title_index: usize, exit_code: i32 },
}

// Process launch broker supervising game execution and lifecycle states.
pub struct ProcessLaunchBroker {
    state: LaunchState,
    next_pid: u32,
    launch_count: u64,
    last_exit_code: Option<i32>,
}

impl ProcessLaunchBroker {
    // Construct an idle process launch broker.
    pub const fn new() -> Self {
        Self {
            state: LaunchState::Idle,
            next_pid: 2000,
            launch_count: 0,
            last_exit_code: None,
        }
    }

    // Return the current execution state.
    pub fn state(&self) -> &LaunchState {
        &self.state
    }

    // Return active process PID if running or suspended.
    pub fn active_pid(&self) -> Option<u32> {
        match self.state {
            LaunchState::Running { pid, .. } | LaunchState::Suspended { pid, .. } => Some(pid),
            _ => None,
        }
    }

    // Return active game title index if launched.
    pub fn active_title_index(&self) -> Option<usize> {
        match self.state {
            LaunchState::Preparing { title_index }
            | LaunchState::Running { title_index, .. }
            | LaunchState::Suspended { title_index, .. }
            | LaunchState::Terminated { title_index, .. } => Some(title_index),
            LaunchState::Idle => None,
        }
    }

    // Check if a game process is currently running.
    pub fn is_game_running(&self) -> bool {
        matches!(self.state, LaunchState::Running { .. })
    }

    // Request execution launch of a game title from catalog.
    pub fn request_launch(
        &mut self,
        title_index: usize,
        catalog: &GameLibraryCatalog,
        start_tick: u64,
    ) -> Result<u32, &'static str> {
        if self.is_game_running() {
            return Err("Another process is currently running");
        }
        if catalog.get_game(title_index).is_none() {
            return Err("Invalid game title index");
        }

        self.state = LaunchState::Preparing { title_index };
        let pid = self.next_pid;
        self.next_pid = self.next_pid.wrapping_add(1);
        self.launch_count += 1;

        self.state = LaunchState::Running {
            pid,
            title_index,
            start_tick,
        };
        Ok(pid)
    }

    // Suspend active game process for quick hardware drawer overlay or system pause.
    pub fn suspend(&mut self) -> Result<(), &'static str> {
        match self.state {
            LaunchState::Running { pid, title_index, .. } => {
                self.state = LaunchState::Suspended { pid, title_index };
                Ok(())
            }
            _ => Err("Process is not in running state"),
        }
    }

    // Resume suspended game execution.
    pub fn resume(&mut self, current_tick: u64) -> Result<(), &'static str> {
        match self.state {
            LaunchState::Suspended { pid, title_index } => {
                self.state = LaunchState::Running {
                    pid,
                    title_index,
                    start_tick: current_tick,
                };
                Ok(())
            }
            _ => Err("Process is not in suspended state"),
        }
    }

    // Terminate running or suspended game and update play time.
    pub fn terminate(
        &mut self,
        exit_code: i32,
        catalog: &mut GameLibraryCatalog,
        current_tick: u64,
    ) -> Result<(), &'static str> {
        match self.state {
            LaunchState::Running { pid, title_index, start_tick } => {
                let duration = current_tick.saturating_sub(start_tick);
                if let Some(meta) = catalog.get_game_mut(title_index) {
                    meta.add_play_time(duration);
                }
                self.last_exit_code = Some(exit_code);
                self.state = LaunchState::Terminated {
                    pid,
                    title_index,
                    exit_code,
                };
                Ok(())
            }
            LaunchState::Suspended { pid, title_index } => {
                self.last_exit_code = Some(exit_code);
                self.state = LaunchState::Terminated {
                    pid,
                    title_index,
                    exit_code,
                };
                Ok(())
            }
            _ => Err("No active process to terminate"),
        }
    }

    // Return the total number of game launches initiated.
    pub fn total_launches(&self) -> u64 {
        self.launch_count
    }
}


