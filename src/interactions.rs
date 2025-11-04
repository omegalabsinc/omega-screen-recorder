use crate::error::{Result, ScreenRecError};
use rdev::{listen, Event, EventType, Key};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Represents a mouse event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseEvent {
    /// Timestamp in milliseconds from recording start
    pub timestamp_ms: u64,
    /// X coordinate
    pub x: f64,
    /// Y coordinate
    pub y: f64,
    /// Event type: move, click, scroll, etc.
    pub event_type: String,
    /// Button info (for clicks): left, right, middle
    pub button: Option<String>,
}

/// Represents a keyboard event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyboardEvent {
    /// Timestamp in milliseconds from recording start
    pub timestamp_ms: u64,
    /// Key name
    pub key: String,
    /// Event type: press or release
    pub event_type: String,
}

/// Complete interaction data for a recording session
#[derive(Debug, Serialize, Deserialize)]
pub struct InteractionData {
    /// Recording duration in milliseconds
    pub duration_ms: u64,
    /// Screen resolution
    pub screen_width: usize,
    pub screen_height: usize,
    /// All mouse events
    pub mouse_events: Vec<MouseEvent>,
    /// All keyboard events
    pub keyboard_events: Vec<KeyboardEvent>,
    /// Metadata
    pub metadata: InteractionMetadata,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InteractionMetadata {
    /// When the recording started
    pub started_at: String,
    /// Total mouse movements captured
    pub total_mouse_moves: usize,
    /// Total mouse clicks captured
    pub total_mouse_clicks: usize,
    /// Total keyboard events captured
    pub total_keyboard_events: usize,
}

/// Interaction tracker that captures mouse and keyboard events
#[derive(Clone)]
pub struct InteractionTracker {
    start_time: Arc<Instant>,
    mouse_events: Arc<Mutex<Vec<MouseEvent>>>,
    keyboard_events: Arc<Mutex<Vec<KeyboardEvent>>>,
    screen_width: usize,
    screen_height: usize,
    track_movements: bool,
    movement_sample_rate: usize, // Capture every Nth movement to avoid too much data
}

impl InteractionTracker {
    pub fn new(screen_width: usize, screen_height: usize, track_movements: bool) -> Self {
        Self {
            start_time: Arc::new(Instant::now()),
            mouse_events: Arc::new(Mutex::new(Vec::new())),
            keyboard_events: Arc::new(Mutex::new(Vec::new())),
            screen_width,
            screen_height,
            track_movements,
            movement_sample_rate: 5, // Capture every 5th movement event
        }
    }

    /// Start listening for mouse and keyboard events
    pub fn start(&self) -> Result<std::thread::JoinHandle<()>> {
        let mouse_events = Arc::clone(&self.mouse_events);
        let keyboard_events = Arc::clone(&self.keyboard_events);
        let start_time = Arc::clone(&self.start_time);
        let track_movements = self.track_movements;
        let mut movement_counter = 0usize;
        let movement_sample_rate = self.movement_sample_rate;

        log::info!("Starting interaction tracking...");
        log::info!("  Track mouse movements: {}", track_movements);
        log::info!("  Movement sample rate: 1/{}", movement_sample_rate);

        // Spawn a thread to listen for events
        let handle = std::thread::spawn(move || {
            let callback = move |event: Event| {
                let timestamp_ms = start_time.elapsed().as_millis() as u64;

                match event.event_type {
                    EventType::MouseMove { x, y } => {
                        if track_movements {
                            movement_counter += 1;
                            // Only capture every Nth movement to reduce data volume
                            if movement_counter % movement_sample_rate == 0 {
                                let mouse_event = MouseEvent {
                                    timestamp_ms,
                                    x,
                                    y,
                                    event_type: "move".to_string(),
                                    button: None,
                                };
                                if let Ok(mut events) = mouse_events.lock() {
                                    events.push(mouse_event);
                                }
                            }
                        }
                    }
                    EventType::ButtonPress(button) => {
                        let button_name = format!("{:?}", button).to_lowercase();
                        if let Some((x, y)) = get_mouse_position() {
                            let mouse_event = MouseEvent {
                                timestamp_ms,
                                x,
                                y,
                                event_type: "click".to_string(),
                                button: Some(button_name),
                            };
                            if let Ok(mut events) = mouse_events.lock() {
                                events.push(mouse_event);
                            }
                        }
                    }
                    EventType::ButtonRelease(button) => {
                        let button_name = format!("{:?}", button).to_lowercase();
                        if let Some((x, y)) = get_mouse_position() {
                            let mouse_event = MouseEvent {
                                timestamp_ms,
                                x,
                                y,
                                event_type: "release".to_string(),
                                button: Some(button_name),
                            };
                            if let Ok(mut events) = mouse_events.lock() {
                                events.push(mouse_event);
                            }
                        }
                    }
                    EventType::Wheel { delta_x, delta_y } => {
                        if let Some((x, y)) = get_mouse_position() {
                            let mouse_event = MouseEvent {
                                timestamp_ms,
                                x,
                                y,
                                event_type: format!("scroll({}, {})", delta_x, delta_y),
                                button: None,
                            };
                            if let Ok(mut events) = mouse_events.lock() {
                                events.push(mouse_event);
                            }
                        }
                    }
                    EventType::KeyPress(key) => {
                        let key_name = format_key(key);
                        let keyboard_event = KeyboardEvent {
                            timestamp_ms,
                            key: key_name,
                            event_type: "press".to_string(),
                        };
                        if let Ok(mut events) = keyboard_events.lock() {
                            events.push(keyboard_event);
                        }
                    }
                    EventType::KeyRelease(key) => {
                        let key_name = format_key(key);
                        let keyboard_event = KeyboardEvent {
                            timestamp_ms,
                            key: key_name,
                            event_type: "release".to_string(),
                        };
                        if let Ok(mut events) = keyboard_events.lock() {
                            events.push(keyboard_event);
                        }
                    }
                }
            };

            // Start listening for events (this blocks)
            if let Err(error) = listen(callback) {
                log::error!("Error listening for events: {:?}", error);
            }
        });

        Ok(handle)
    }

    /// Save interaction data to a JSON file
    pub fn save(&self, output_path: &Path) -> Result<()> {
        let duration_ms = self.start_time.elapsed().as_millis() as u64;

        let mouse_events = self
            .mouse_events
            .lock()
            .map_err(|e| ScreenRecError::ConfigError(format!("Failed to lock mouse events: {}", e)))?
            .clone();

        let keyboard_events = self
            .keyboard_events
            .lock()
            .map_err(|e| ScreenRecError::ConfigError(format!("Failed to lock keyboard events: {}", e)))?
            .clone();

        // Count different event types
        let total_mouse_moves = mouse_events
            .iter()
            .filter(|e| e.event_type == "move")
            .count();
        let total_mouse_clicks = mouse_events
            .iter()
            .filter(|e| e.event_type == "click")
            .count();
        let total_keyboard_events = keyboard_events.len();

        let interaction_data = InteractionData {
            duration_ms,
            screen_width: self.screen_width,
            screen_height: self.screen_height,
            mouse_events,
            keyboard_events,
            metadata: InteractionMetadata {
                started_at: chrono::Local::now().to_rfc3339(),
                total_mouse_moves,
                total_mouse_clicks,
                total_keyboard_events,
            },
        };

        // Serialize to JSON
        let json = serde_json::to_string_pretty(&interaction_data).map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to serialize interaction data: {}", e))
        })?;

        // Write to file
        let mut file = File::create(output_path)?;
        file.write_all(json.as_bytes())?;

        log::info!("Interaction data saved to: {:?}", output_path);
        log::info!("  Duration: {:.2}s", duration_ms as f64 / 1000.0);
        log::info!("  Mouse movements: {}", total_mouse_moves);
        log::info!("  Mouse clicks: {}", total_mouse_clicks);
        log::info!("  Keyboard events: {}", total_keyboard_events);

        Ok(())
    }
}

/// Get current mouse position (platform-independent)
fn get_mouse_position() -> Option<(f64, f64)> {
    // rdev doesn't provide a direct way to get mouse position
    // This is a limitation - we rely on move events to track position
    None
}

/// Format a key for display
fn format_key(key: Key) -> String {
    match key {
        Key::Alt => "Alt".to_string(),
        Key::AltGr => "AltGr".to_string(),
        Key::Backspace => "Backspace".to_string(),
        Key::CapsLock => "CapsLock".to_string(),
        Key::ControlLeft => "CtrlLeft".to_string(),
        Key::ControlRight => "CtrlRight".to_string(),
        Key::Delete => "Delete".to_string(),
        Key::DownArrow => "Down".to_string(),
        Key::End => "End".to_string(),
        Key::Escape => "Esc".to_string(),
        Key::F1 => "F1".to_string(),
        Key::F2 => "F2".to_string(),
        Key::F3 => "F3".to_string(),
        Key::F4 => "F4".to_string(),
        Key::F5 => "F5".to_string(),
        Key::F6 => "F6".to_string(),
        Key::F7 => "F7".to_string(),
        Key::F8 => "F8".to_string(),
        Key::F9 => "F9".to_string(),
        Key::F10 => "F10".to_string(),
        Key::F11 => "F11".to_string(),
        Key::F12 => "F12".to_string(),
        Key::Home => "Home".to_string(),
        Key::LeftArrow => "Left".to_string(),
        Key::MetaLeft => "MetaLeft".to_string(),
        Key::MetaRight => "MetaRight".to_string(),
        Key::PageDown => "PageDown".to_string(),
        Key::PageUp => "PageUp".to_string(),
        Key::Return => "Enter".to_string(),
        Key::RightArrow => "Right".to_string(),
        Key::ShiftLeft => "ShiftLeft".to_string(),
        Key::ShiftRight => "ShiftRight".to_string(),
        Key::Space => "Space".to_string(),
        Key::Tab => "Tab".to_string(),
        Key::UpArrow => "Up".to_string(),
        Key::PrintScreen => "PrintScreen".to_string(),
        Key::ScrollLock => "ScrollLock".to_string(),
        Key::Pause => "Pause".to_string(),
        Key::NumLock => "NumLock".to_string(),
        Key::BackQuote => "`".to_string(),
        Key::Num1 => "1".to_string(),
        Key::Num2 => "2".to_string(),
        Key::Num3 => "3".to_string(),
        Key::Num4 => "4".to_string(),
        Key::Num5 => "5".to_string(),
        Key::Num6 => "6".to_string(),
        Key::Num7 => "7".to_string(),
        Key::Num8 => "8".to_string(),
        Key::Num9 => "9".to_string(),
        Key::Num0 => "0".to_string(),
        Key::Minus => "-".to_string(),
        Key::Equal => "=".to_string(),
        Key::KeyQ => "Q".to_string(),
        Key::KeyW => "W".to_string(),
        Key::KeyE => "E".to_string(),
        Key::KeyR => "R".to_string(),
        Key::KeyT => "T".to_string(),
        Key::KeyY => "Y".to_string(),
        Key::KeyU => "U".to_string(),
        Key::KeyI => "I".to_string(),
        Key::KeyO => "O".to_string(),
        Key::KeyP => "P".to_string(),
        Key::LeftBracket => "[".to_string(),
        Key::RightBracket => "]".to_string(),
        Key::KeyA => "A".to_string(),
        Key::KeyS => "S".to_string(),
        Key::KeyD => "D".to_string(),
        Key::KeyF => "F".to_string(),
        Key::KeyG => "G".to_string(),
        Key::KeyH => "H".to_string(),
        Key::KeyJ => "J".to_string(),
        Key::KeyK => "K".to_string(),
        Key::KeyL => "L".to_string(),
        Key::SemiColon => ";".to_string(),
        Key::Quote => "'".to_string(),
        Key::BackSlash => "\\".to_string(),
        Key::IntlBackslash => "IntlBackslash".to_string(),
        Key::KeyZ => "Z".to_string(),
        Key::KeyX => "X".to_string(),
        Key::KeyC => "C".to_string(),
        Key::KeyV => "V".to_string(),
        Key::KeyB => "B".to_string(),
        Key::KeyN => "N".to_string(),
        Key::KeyM => "M".to_string(),
        Key::Comma => ",".to_string(),
        Key::Dot => ".".to_string(),
        Key::Slash => "/".to_string(),
        Key::Insert => "Insert".to_string(),
        Key::KpReturn => "KpEnter".to_string(),
        Key::KpMinus => "Kp-".to_string(),
        Key::KpPlus => "Kp+".to_string(),
        Key::KpMultiply => "Kp*".to_string(),
        Key::KpDivide => "Kp/".to_string(),
        Key::Kp0 => "Kp0".to_string(),
        Key::Kp1 => "Kp1".to_string(),
        Key::Kp2 => "Kp2".to_string(),
        Key::Kp3 => "Kp3".to_string(),
        Key::Kp4 => "Kp4".to_string(),
        Key::Kp5 => "Kp5".to_string(),
        Key::Kp6 => "Kp6".to_string(),
        Key::Kp7 => "Kp7".to_string(),
        Key::Kp8 => "Kp8".to_string(),
        Key::Kp9 => "Kp9".to_string(),
        Key::KpDelete => "KpDelete".to_string(),
        Key::Function => "Function".to_string(),
        Key::Unknown(code) => format!("Unknown({})", code),
    }
}
