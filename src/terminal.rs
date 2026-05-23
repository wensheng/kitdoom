use std::{
    io::{self, Write},
    panic,
};

use anyhow::Result;
use crossterm::{
    Command,
    cursor::{Hide, MoveTo, Show},
    event::{
        DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode, size, window_size,
    },
};

pub const DOOM_WIDTH: u32 = 640;
pub const DOOM_HEIGHT: u32 = 400;

pub struct TerminalSession;

impl TerminalSession {
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(
            stdout,
            EnterAlternateScreen,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            ),
            EnableMouseCapture,
            EnableSgrPixelMouse,
            Hide,
            Clear(ClearType::All),
            MoveTo(0, 0)
        ) {
            restore_terminal();
            return Err(error.into());
        }
        stdout.flush()?;
        Ok(Self)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        restore_terminal();
    }
}

pub fn restore_terminal() {
    let mut stdout = io::stdout();
    let _ = execute!(
        stdout,
        EndSynchronizedUpdate,
        Show,
        DisableSgrPixelMouse,
        DisableMouseCapture,
        PopKeyboardEnhancementFlags,
        LeaveAlternateScreen,
        MoveTo(0, 0)
    );
    let _ = stdout.flush();
    let _ = disable_raw_mode();
}

pub fn install_panic_restore_hook() {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameLayout {
    pub terminal_cols: u16,
    pub terminal_rows: u16,
    pub display_width_px: u32,
    pub display_height_px: u32,
    pub cell_width_px: f32,
    pub cell_height_px: f32,
    pub image_cols: u16,
    pub image_rows: u16,
}

impl FrameLayout {
    pub fn current(scale: bool) -> Self {
        let (cols, rows) = size().unwrap_or((80, 24));
        let window = window_size().ok();
        let width_px = window
            .as_ref()
            .filter(|window| window.width > 0 && window.height > 0)
            .map(|window| u32::from(window.width))
            .unwrap_or_else(|| u32::from(cols.max(1)) * 10);
        let height_px = window
            .as_ref()
            .filter(|window| window.width > 0 && window.height > 0)
            .map(|window| u32::from(window.height))
            .unwrap_or_else(|| u32::from(rows.max(1)) * 20);

        Self::from_dimensions(cols, rows, width_px, height_px, scale)
    }

    pub fn from_dimensions(
        terminal_cols: u16,
        terminal_rows: u16,
        display_width_px: u32,
        display_height_px: u32,
        scale: bool,
    ) -> Self {
        let terminal_cols = terminal_cols.max(1);
        let terminal_rows = terminal_rows.max(1);
        let display_width_px = display_width_px.max(1);
        let display_height_px = display_height_px.max(1);
        let cell_width_px = display_width_px as f32 / f32::from(terminal_cols);
        let cell_height_px = display_height_px as f32 / f32::from(terminal_rows);
        let (image_cols, image_rows) = if scale {
            scaled_cells(
                terminal_cols,
                terminal_rows,
                display_width_px as f32,
                display_height_px as f32,
                cell_width_px,
                cell_height_px,
            )
        } else {
            natural_cells(terminal_cols, terminal_rows, cell_width_px, cell_height_px)
        };

        Self {
            terminal_cols,
            terminal_rows,
            display_width_px,
            display_height_px,
            cell_width_px,
            cell_height_px,
            image_cols,
            image_rows,
        }
    }

    pub fn image_width_px(self) -> f32 {
        f32::from(self.image_cols) * self.cell_width_px
    }

    pub fn image_height_px(self) -> f32 {
        f32::from(self.image_rows) * self.cell_height_px
    }

    pub fn mouse_position_px(self, column: u16, row: u16) -> (f32, f32) {
        if column >= self.terminal_cols || row >= self.terminal_rows {
            (f32::from(column), f32::from(row))
        } else {
            (
                f32::from(column) * self.cell_width_px,
                f32::from(row) * self.cell_height_px,
            )
        }
    }
}

fn scaled_cells(
    terminal_cols: u16,
    terminal_rows: u16,
    display_width_px: f32,
    display_height_px: f32,
    cell_width_px: f32,
    cell_height_px: f32,
) -> (u16, u16) {
    let aspect = DOOM_WIDTH as f32 / DOOM_HEIGHT as f32;
    let (width_px, height_px) = if display_width_px / aspect <= display_height_px {
        (display_width_px, display_width_px / aspect)
    } else {
        (display_height_px * aspect, display_height_px)
    };
    let cols = (width_px / cell_width_px).floor().max(1.0) as u16;
    let rows = (height_px / cell_height_px).floor().max(1.0) as u16;
    (cols.min(terminal_cols), rows.min(terminal_rows))
}

fn natural_cells(
    terminal_cols: u16,
    terminal_rows: u16,
    cell_width_px: f32,
    cell_height_px: f32,
) -> (u16, u16) {
    let cols = (DOOM_WIDTH as f32 / cell_width_px).ceil().max(1.0) as u16;
    let rows = (DOOM_HEIGHT as f32 / cell_height_px).ceil().max(1.0) as u16;
    (cols.min(terminal_cols), rows.min(terminal_rows))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BeginSynchronizedUpdate;

impl Command for BeginSynchronizedUpdate {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        f.write_str("\x1b[?2026h")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndSynchronizedUpdate;

impl Command for EndSynchronizedUpdate {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        f.write_str("\x1b[?2026l")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EnableSgrPixelMouse;

impl Command for EnableSgrPixelMouse {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        f.write_str("\x1b[?1016h")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DisableSgrPixelMouse;

impl Command for DisableSgrPixelMouse {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        f.write_str("\x1b[?1016l")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_layout_preserves_doom_aspect() {
        let layout = FrameLayout::from_dimensions(100, 40, 1000, 800, true);

        assert_eq!(layout.image_cols, 100);
        assert_eq!(layout.image_rows, 31);
        let aspect = layout.image_width_px() / layout.image_height_px();
        assert!((aspect - 1.6).abs() < 0.04);
    }

    #[test]
    fn scaled_layout_uses_height_when_limited() {
        let layout = FrameLayout::from_dimensions(200, 30, 2000, 600, true);

        assert_eq!(layout.image_rows, 30);
        assert_eq!(layout.image_cols, 96);
        let aspect = layout.image_width_px() / layout.image_height_px();
        assert!((aspect - 1.6).abs() < 0.04);
    }

    #[test]
    fn natural_layout_uses_doom_pixel_size() {
        let layout = FrameLayout::from_dimensions(100, 40, 1000, 800, false);

        assert_eq!(layout.image_cols, 64);
        assert_eq!(layout.image_rows, 20);
    }
}
