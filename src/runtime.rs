use std::{
    ffi::{CStr, CString},
    io::{self, Write},
    os::raw::{c_char, c_int, c_uint},
    ptr,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use crossterm::{
    cursor::MoveTo,
    event::{
        self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    queue,
    terminal::{Clear, ClearType},
};
use signal_hook::{
    consts::signal::{SIGINT, SIGTERM},
    iterator::Signals,
};

use crate::{
    input,
    kitty::{flush_robust, write_rgb_frame},
    terminal::{self, BeginSynchronizedUpdate, EndSynchronizedUpdate, FrameLayout},
};

const DOOM_WIDTH: usize = terminal::DOOM_WIDTH as usize;
const DOOM_HEIGHT: usize = terminal::DOOM_HEIGHT as usize;
const DOOM_PIXELS: usize = DOOM_WIDTH * DOOM_HEIGHT;
const RGB_FRAME_BYTES: usize = DOOM_PIXELS * 3;
const KEY_QUEUE_LEN: usize = 32;

static STATE: OnceLock<Mutex<RuntimeState>> = OnceLock::new();
static EXIT_FLAG: AtomicBool = AtomicBool::new(false);
static SIGNAL_HANDLERS_INSTALLED: AtomicBool = AtomicBool::new(false);

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
enum DoomEventType {
    KeyDown = 0,
    KeyUp = 1,
    Mouse = 2,
    Joystick = 3,
    Quit = 4,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct DoomEvent {
    event_type: DoomEventType,
    data1: c_int,
    data2: c_int,
    data3: c_int,
    data4: c_int,
}

unsafe extern "C" {
    static mut DG_ScreenBuffer: *mut u32;

    fn doomgeneric_Create(argc: c_int, argv: *mut *mut c_char);
    fn doomgeneric_Tick();
    fn D_PostEvent(ev: *mut DoomEvent);
}

#[derive(Debug)]
struct RuntimeState {
    startup: Instant,
    key_queue: [u16; KEY_QUEUE_LEN],
    key_queue_write_idx: usize,
    key_queue_read_idx: usize,
    mouse_enabled: bool,
    scale: bool,
    last_mouse_position: Option<(f32, f32)>,
    mouse_buttons: c_int,
    rgb_frame: Vec<u8>,
}

impl RuntimeState {
    fn new() -> Self {
        Self {
            startup: Instant::now(),
            key_queue: [0; KEY_QUEUE_LEN],
            key_queue_write_idx: 0,
            key_queue_read_idx: 0,
            mouse_enabled: true,
            scale: true,
            last_mouse_position: None,
            mouse_buttons: 0,
            rgb_frame: vec![0; RGB_FRAME_BYTES],
        }
    }

    fn enqueue_key(&mut self, pressed: bool, doom_key: u8) {
        let key_data = (u16::from(pressed) << 8) | u16::from(doom_key);
        self.key_queue[self.key_queue_write_idx] = key_data;
        self.key_queue_write_idx = (self.key_queue_write_idx + 1) % KEY_QUEUE_LEN;
    }

    fn pop_key(&mut self) -> Option<(c_int, u8)> {
        if self.key_queue_read_idx == self.key_queue_write_idx {
            return None;
        }

        let key_data = self.key_queue[self.key_queue_read_idx];
        self.key_queue_read_idx = (self.key_queue_read_idx + 1) % KEY_QUEUE_LEN;
        Some(((key_data >> 8) as c_int, (key_data & 0xff) as u8))
    }
}

pub fn reset_exit_request() {
    EXIT_FLAG.store(false, Ordering::SeqCst);
}

pub fn request_exit() {
    EXIT_FLAG.store(true, Ordering::SeqCst);
}

pub fn install_signal_handlers() -> Result<()> {
    if SIGNAL_HANDLERS_INSTALLED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Ok(());
    }

    let mut signals =
        Signals::new([SIGINT, SIGTERM]).context("failed to install signal handlers")?;

    if let Err(error) = thread::Builder::new()
        .name("kitdoom-signals".to_string())
        .spawn(move || {
            for _ in signals.forever() {
                request_exit();
            }
        })
    {
        SIGNAL_HANDLERS_INSTALLED.store(false, Ordering::SeqCst);
        return Err(error).context("failed to spawn signal handler thread");
    }

    Ok(())
}

pub fn run(c_args: &mut [CString]) -> Result<()> {
    if STATE.set(Mutex::new(RuntimeState::new())).is_err() {
        bail!("Doom runtime has already been initialized");
    }

    let mut argv: Vec<*mut c_char> = c_args
        .iter_mut()
        .map(|arg| arg.as_ptr() as *mut c_char)
        .collect();

    unsafe {
        doomgeneric_Create(argv.len() as c_int, argv.as_mut_ptr());
    }

    while !EXIT_FLAG.load(Ordering::SeqCst) {
        unsafe {
            doomgeneric_Tick();
        }
    }

    Ok(())
}

#[unsafe(no_mangle)]
pub extern "C" fn DG_Init() {}

#[unsafe(no_mangle)]
pub extern "C" fn DG_DrawFrame() {
    if let Err(error) = draw_frame() {
        let _ = writeln!(io::stderr(), "render error: {error}");
        request_exit();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn DG_SleepMs(ms: c_uint) {
    thread::sleep(Duration::from_millis(u64::from(ms)));
}

#[unsafe(no_mangle)]
pub extern "C" fn DG_GetTicksMs() -> u32 {
    with_state_mut(|state| state.startup.elapsed().as_millis() as u32).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn DG_GetKey(pressed: *mut c_int, doom_key: *mut u8) -> c_int {
    with_state_mut(|state| {
        drain_terminal_events(state);
        if let Some((is_pressed, key)) = state.pop_key() {
            unsafe {
                if !pressed.is_null() {
                    *pressed = is_pressed;
                }
                if !doom_key.is_null() {
                    *doom_key = key;
                }
            }
            1
        } else {
            0
        }
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn DG_SetWindowTitle(title: *const c_char) {
    if title.is_null() {
        return;
    }

    let Ok(title) = unsafe { CStr::from_ptr(title) }.to_str() else {
        return;
    };
    let mut stdout = io::stdout();
    let _ = write!(stdout, "\x1b]0;{title}\x07");
    let _ = stdout.flush();
}

#[unsafe(no_mangle)]
pub extern "C" fn DG_ErrorExit() {
    EXIT_FLAG.store(true, Ordering::SeqCst);
    terminal::restore_terminal();
}

fn draw_frame() -> Result<()> {
    with_state_mut(|state| {
        drain_terminal_events(state);
        convert_doom_pixels_from_global(&mut state.rgb_frame);

        let layout = FrameLayout::current(state.scale);
        let mut stdout = io::stdout().lock();
        queue!(
            stdout,
            BeginSynchronizedUpdate,
            MoveTo(0, 0),
            Clear(ClearType::All)
        )?;
        write_rgb_frame(
            &mut stdout,
            &state.rgb_frame,
            terminal::DOOM_WIDTH,
            terminal::DOOM_HEIGHT,
            u32::from(layout.image_cols),
            u32::from(layout.image_rows),
            true,
        )?;
        queue!(stdout, EndSynchronizedUpdate)?;
        flush_robust(&mut stdout)?;

        drain_terminal_events(state);
        Ok(())
    })
    .unwrap_or_else(|| Ok(()))
}

fn with_state_mut<T>(f: impl FnOnce(&mut RuntimeState) -> T) -> Option<T> {
    let state = STATE.get()?;
    let mut guard = state.lock().ok()?;
    Some(f(&mut guard))
}

fn drain_terminal_events(state: &mut RuntimeState) {
    loop {
        match event::poll(Duration::ZERO) {
            Ok(true) => match event::read() {
                Ok(Event::Key(key)) => handle_key_event(state, key),
                Ok(Event::Mouse(mouse)) => handle_mouse_event(state, mouse),
                Ok(Event::Resize(_, _)) => {
                    state.last_mouse_position = None;
                }
                Ok(_) => {}
                Err(_) => break,
            },
            Ok(false) | Err(_) => break,
        }
    }
}

fn handle_key_event(state: &mut RuntimeState, key: KeyEvent) {
    let pressed = match key.kind {
        KeyEventKind::Press | KeyEventKind::Repeat => true,
        KeyEventKind::Release => false,
    };

    if is_interrupt_key(&key) {
        request_exit();
        return;
    }

    if key.kind == KeyEventKind::Release {
        match key.code {
            KeyCode::Char('m') | KeyCode::Char('M') => {
                state.mouse_enabled = !state.mouse_enabled;
                state.last_mouse_position = None;
                return;
            }
            KeyCode::Char('u') | KeyCode::Char('U') => {
                state.scale = !state.scale;
                state.last_mouse_position = None;
                return;
            }
            _ => {}
        }
    }

    if let Some(doom_key) = input::doom_key_for(key) {
        state.enqueue_key(pressed, doom_key);
    }
}

fn is_interrupt_key(key: &KeyEvent) -> bool {
    let pressed = matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat);
    let ctrl_c = matches!(key.code, KeyCode::Char('c' | 'C'))
        && key.modifiers.contains(KeyModifiers::CONTROL);
    let raw_etx = matches!(key.code, KeyCode::Char('\u{3}'));

    pressed && (ctrl_c || raw_etx)
}

fn handle_mouse_event(state: &mut RuntimeState, mouse: crossterm::event::MouseEvent) {
    if !state.mouse_enabled {
        state.last_mouse_position = None;
        return;
    }

    let layout = FrameLayout::current(state.scale);
    let (x, y) = layout.mouse_position_px(mouse.column, mouse.row);
    let (rel_x, rel_y) = match state.last_mouse_position {
        Some((last_x, last_y)) => ((x - last_x) as c_int, (y - last_y) as c_int),
        None => (0, 0),
    };
    state.last_mouse_position = Some((x, y));

    match mouse.kind {
        MouseEventKind::Down(button) | MouseEventKind::Drag(button) => {
            state.mouse_buttons |= mouse_button_bit(button);
        }
        MouseEventKind::Up(button) => {
            state.mouse_buttons &= !mouse_button_bit(button);
        }
        MouseEventKind::Moved => {}
        MouseEventKind::ScrollDown
        | MouseEventKind::ScrollUp
        | MouseEventKind::ScrollLeft
        | MouseEventKind::ScrollRight => return,
    }

    let scale_x = terminal::DOOM_WIDTH as f32 / layout.image_width_px().max(1.0);
    let scale_y = terminal::DOOM_HEIGHT as f32 / layout.image_height_px().max(1.0);
    let doom_rel_x = ((rel_x as f32) * scale_x) as c_int;
    let doom_rel_y = ((rel_y as f32) * scale_y) as c_int;

    let mut event = DoomEvent {
        event_type: DoomEventType::Mouse,
        data1: state.mouse_buttons,
        data2: accelerate_mouse(doom_rel_x, 16.0),
        data3: -accelerate_mouse(doom_rel_y, 4.0),
        data4: 0,
    };

    unsafe {
        D_PostEvent(ptr::addr_of_mut!(event));
    }
}

fn mouse_button_bit(button: MouseButton) -> c_int {
    match button {
        MouseButton::Left => 1,
        MouseButton::Right => 2,
        MouseButton::Middle => 4,
    }
}

fn accelerate_mouse(delta: c_int, clamp: f32) -> c_int {
    let dx = delta as f32;
    (dx * clamp.min(8.0 * dx.abs().exp())) as c_int
}

fn convert_doom_pixels_from_global(output: &mut [u8]) {
    let screen = unsafe {
        let ptr = DG_ScreenBuffer;
        if ptr.is_null() {
            return;
        }
        std::slice::from_raw_parts(ptr, DOOM_PIXELS)
    };
    convert_doom_pixels(screen, output);
}

pub fn convert_doom_pixels(input: &[u32], output: &mut [u8]) {
    for (pixel, rgb) in input.iter().zip(output.chunks_exact_mut(3)) {
        rgb[0] = ((pixel >> 16) & 0xff) as u8;
        rgb[1] = ((pixel >> 8) & 0xff) as u8;
        rgb[2] = (pixel & 0xff) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_packed_doom_pixels_to_rgb() {
        let input = [0x0012_3456, 0x00ab_cdef];
        let mut output = [0; 6];

        convert_doom_pixels(&input, &mut output);

        assert_eq!(output, [0x12, 0x34, 0x56, 0xab, 0xcd, 0xef]);
    }

    #[test]
    fn detects_control_c_interrupt() {
        assert!(is_interrupt_key(&KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));
        assert!(is_interrupt_key(&KeyEvent::new(
            KeyCode::Char('C'),
            KeyModifiers::CONTROL,
        )));
    }

    #[test]
    fn detects_raw_etx_interrupt() {
        assert!(is_interrupt_key(&KeyEvent::new(
            KeyCode::Char('\u{3}'),
            KeyModifiers::NONE,
        )));
    }

    #[test]
    fn ignores_plain_c() {
        assert!(!is_interrupt_key(&KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::NONE,
        )));
    }
}
