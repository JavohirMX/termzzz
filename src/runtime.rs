use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton as CrosstermMouseButton, MouseEvent, MouseEventKind,
};
use std::io;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    CtrlC,
    Escape,
    Enter,
    Space,
    Backspace,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    Insert,
    Function(u8),
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPhase {
    Pressed,
    Repeated,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerPhase {
    Moved,
    Pressed,
    Released,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    Key {
        key: Key,
        phase: KeyPhase,
    },
    Pointer {
        position: (u16, u16),
        phase: PointerPhase,
        button: PointerButton,
    },
    Resize {
        size: (u16, u16),
    },
    Quit,
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerState {
    pub position: (u16, u16),
    pub previous: Option<(u16, u16)>,
    pub delta: (f32, f32),
    pub normalized: (f32, f32),
    pub pressed: bool,
    pub button: Option<PointerButton>,
    pub wheel_delta: i32,
}

impl Default for PointerState {
    fn default() -> Self {
        Self {
            position: (0, 0),
            previous: None,
            delta: (0.0, 0.0),
            normalized: (0.0, 0.0),
            pressed: false,
            button: None,
            wheel_delta: 0,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct InputState {
    pointer: PointerState,
    pressed_keys: Vec<Key>,
    size: (u16, u16),
}

impl InputState {
    pub fn apply(&mut self, event: InputEvent) {
        match event {
            InputEvent::Key { key, phase } => self.apply_key(key, phase),
            InputEvent::Pointer {
                position,
                phase,
                button,
            } => self.apply_pointer(position, phase, button),
            InputEvent::Resize { size } => self.set_size(size),
            InputEvent::Quit | InputEvent::Ignored => {}
        }
    }

    pub fn begin_frame(&mut self) {
        self.pressed_keys.clear();
    }

    pub fn set_size(&mut self, size: (u16, u16)) {
        self.size = (size.0.max(1), size.1.max(1));
        self.update_normalized_pointer();
    }

    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    pub fn pointer(&self) -> &PointerState {
        &self.pointer
    }

    pub fn is_key_pressed(&self, key: Key) -> bool {
        self.pressed_keys.contains(&key)
    }

    pub fn pressed_keys(&self) -> &[Key] {
        &self.pressed_keys
    }

    fn apply_key(&mut self, key: Key, phase: KeyPhase) {
        match phase {
            KeyPhase::Pressed => {
                if !self.pressed_keys.contains(&key) {
                    self.pressed_keys.push(key);
                }
            }
            KeyPhase::Repeated => {}
            KeyPhase::Released => {
                self.pressed_keys.retain(|pressed| *pressed != key)
            }
        }
    }

    fn apply_pointer(
        &mut self,
        position: (u16, u16),
        phase: PointerPhase,
        button: PointerButton,
    ) {
        let previous = self.pointer.position;
        self.pointer.previous = Some(previous);
        self.pointer.position = position;
        self.pointer.delta = (
            position.0 as f32 - previous.0 as f32,
            position.1 as f32 - previous.1 as f32,
        );

        match phase {
            PointerPhase::Pressed => {
                self.pointer.pressed = true;
                self.pointer.button = Some(button);
            }
            PointerPhase::Moved => {
                if self.pointer.pressed {
                    self.pointer.button = Some(button);
                }
            }
            PointerPhase::Released => {
                self.pointer.pressed = false;
                self.pointer.button = None;
            }
            PointerPhase::WheelUp => self.pointer.wheel_delta += 1,
            PointerPhase::WheelDown => self.pointer.wheel_delta -= 1,
            PointerPhase::WheelLeft | PointerPhase::WheelRight => {}
        }

        self.update_normalized_pointer();
    }

    fn update_normalized_pointer(&mut self) {
        let width = self.size.0.max(1) as f32;
        let height = self.size.1.max(1) as f32;
        self.pointer.normalized = (
            self.pointer.position.0 as f32 / (width - 1.0).max(1.0),
            self.pointer.position.1 as f32 / (height - 1.0).max(1.0),
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameContext {
    pub size: (u16, u16),
    pub frame: u64,
    pub elapsed: Duration,
    pub delta: Duration,
    pub input: InputState,
}

impl FrameContext {
    pub fn new(
        size: (u16, u16),
        frame: u64,
        elapsed: Duration,
        delta: Duration,
        input: InputState,
    ) -> Self {
        Self {
            size,
            frame,
            elapsed,
            delta,
            input,
        }
    }
}

pub trait InputSource {
    fn poll(&mut self, timeout: Duration) -> io::Result<Vec<InputEvent>>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct CrosstermInput;

impl InputSource for CrosstermInput {
    fn poll(&mut self, timeout: Duration) -> io::Result<Vec<InputEvent>> {
        if !event::poll(timeout)? {
            return Ok(Vec::new());
        }

        let mut events = Vec::new();
        loop {
            events.push(translate_event(event::read()?));
            if !event::poll(Duration::from_millis(0))? {
                break;
            }
        }
        Ok(events)
    }
}

fn translate_event(event: Event) -> InputEvent {
    match event {
        Event::Key(key) => translate_key(key),
        Event::Mouse(mouse) => translate_mouse(mouse),
        Event::Resize(width, height) => InputEvent::Resize {
            size: (width, height),
        },
        Event::FocusGained | Event::FocusLost | Event::Paste(_) => {
            InputEvent::Ignored
        }
    }
}

fn translate_key(event: KeyEvent) -> InputEvent {
    let key = if event.code == KeyCode::Char('c')
        && event.modifiers.contains(KeyModifiers::CONTROL)
    {
        Key::CtrlC
    } else {
        match event.code {
            KeyCode::Char(' ') => Key::Space,
            KeyCode::Char(character) => Key::Char(character),
            KeyCode::Esc => Key::Escape,
            KeyCode::Enter => Key::Enter,
            KeyCode::Backspace => Key::Backspace,
            KeyCode::Tab | KeyCode::BackTab => Key::Tab,
            KeyCode::Left => Key::Left,
            KeyCode::Right => Key::Right,
            KeyCode::Up => Key::Up,
            KeyCode::Down => Key::Down,
            KeyCode::Home => Key::Home,
            KeyCode::End => Key::End,
            KeyCode::PageUp => Key::PageUp,
            KeyCode::PageDown => Key::PageDown,
            KeyCode::Delete => Key::Delete,
            KeyCode::Insert => Key::Insert,
            KeyCode::F(number) => Key::Function(number),
            _ => Key::Other,
        }
    };

    let phase = match event.kind {
        KeyEventKind::Press => KeyPhase::Pressed,
        KeyEventKind::Repeat => KeyPhase::Repeated,
        KeyEventKind::Release => KeyPhase::Released,
    };

    InputEvent::Key { key, phase }
}

fn translate_mouse(event: MouseEvent) -> InputEvent {
    let (phase, button) = match event.kind {
        MouseEventKind::Down(button) => {
            (PointerPhase::Pressed, translate_button(button))
        }
        MouseEventKind::Drag(button) => {
            (PointerPhase::Moved, translate_button(button))
        }
        MouseEventKind::Up(button) => {
            (PointerPhase::Released, translate_button(button))
        }
        MouseEventKind::Moved => (PointerPhase::Moved, PointerButton::Left),
        MouseEventKind::ScrollUp => (PointerPhase::WheelUp, PointerButton::Left),
        MouseEventKind::ScrollDown => {
            (PointerPhase::WheelDown, PointerButton::Left)
        }
        MouseEventKind::ScrollLeft => {
            (PointerPhase::WheelLeft, PointerButton::Left)
        }
        MouseEventKind::ScrollRight => {
            (PointerPhase::WheelRight, PointerButton::Left)
        }
    };

    InputEvent::Pointer {
        position: (event.column, event.row),
        phase,
        button,
    }
}

fn translate_button(button: CrosstermMouseButton) -> PointerButton {
    match button {
        CrosstermMouseButton::Left => PointerButton::Left,
        CrosstermMouseButton::Right => PointerButton::Right,
        CrosstermMouseButton::Middle => PointerButton::Middle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_space_to_space_key() {
        let event = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(
            translate_key(event),
            InputEvent::Key {
                key: Key::Space,
                phase: KeyPhase::Pressed,
            }
        );
    }
}
