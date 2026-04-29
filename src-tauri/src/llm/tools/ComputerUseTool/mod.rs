use std::io::Cursor;
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use base64::Engine;
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use screenshots::image::{DynamicImage, ImageFormat};
use screenshots::Screen;
use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::sync::Mutex;

use crate::llm::tools::{
    app_tool_with_extras, AppExecuteFuture, ToolPermissionDescriptor, ToolRegistration,
};
use crate::llm::types::{Content, ContentBlock, ImageSource, Message, Role, Tool};

const SCREENSHOT_MEDIA_TYPE: &str = "image/png";
const DEFAULT_WAIT_MS: u64 = 500;
const MAX_WAIT_MS: u64 = 10_000;
const DEFAULT_DRAG_SETTLE_MS: u64 = 50;

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, conversation_id.as_deref(), input).await })
}

fn permission(input: &Value) -> Option<ToolPermissionDescriptor> {
    let action = input
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .trim();

    Some(ToolPermissionDescriptor {
        signature: "computer_use:session_access".to_string(),
        preview: format!("桌面控制（computer_use:{}）", action),
        warning: Some(
            "该操作会读取屏幕内容并注入鼠标或键盘输入，请仅在确认目标桌面环境安全时授权"
                .to_string(),
        ),
        needs_approval: true,
    })
}

pub(crate) fn registration() -> ToolRegistration {
    app_tool_with_extras(
        tool,
        execute,
        execute_with_app_boxed,
        false,
        Some(permission),
        Some(postprocess_output),
    )
}

fn session_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub fn tool() -> Tool {
    Tool {
        name: "computer_use".into(),
        description: "Control the local desktop with guarded computer-use actions such as request_access, list_displays, cursor_position, screenshot, mouse movement, clicks, scrolling, typing, hotkeys, drag, and waits.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "request_access",
                        "list_displays",
                        "cursor_position",
                        "screenshot",
                        "move_mouse",
                        "click",
                        "double_click",
                        "drag",
                        "scroll",
                        "type_text",
                        "key",
                        "hotkey",
                        "wait"
                    ]
                },
                "display_id": { "type": "integer" },
                "region": {
                    "type": "object",
                    "properties": {
                        "x": { "type": "integer" },
                        "y": { "type": "integer" },
                        "width": { "type": "integer" },
                        "height": { "type": "integer" }
                    },
                    "required": ["x", "y", "width", "height"]
                },
                "x": { "type": "integer" },
                "y": { "type": "integer" },
                "from_x": { "type": "integer" },
                "from_y": { "type": "integer" },
                "to_x": { "type": "integer" },
                "to_y": { "type": "integer" },
                "dx": { "type": "integer" },
                "dy": { "type": "integer" },
                "button": {
                    "type": "string",
                    "enum": ["left", "middle", "right"]
                },
                "count": { "type": "integer" },
                "text": { "type": "string" },
                "key_name": { "type": "string" },
                "key": { "type": "string" },
                "keys": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "duration_ms": { "type": "integer" },
                "ms": { "type": "integer" }
            },
            "required": ["action"]
        }),
    }
}

pub fn execute(input: Value) -> String {
    let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("unknown");
    json!({
        "ok": false,
        "action": action,
        "message": "computer_use requires AppHandle-aware execution and should be routed via execute_tool_with_app."
    })
    .to_string()
}

fn parse_i32_field(input: &Value, key: &str) -> Result<i32, String> {
    input
        .get(key)
        .and_then(|v| v.as_i64())
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| format!("computer_use requires integer '{}'", key))
}

fn parse_u64_field(input: &Value, key: &str) -> Option<u64> {
    input
        .get(key)
        .and_then(|v| v.as_u64())
}

fn parse_button(input: &Value) -> Result<Button, String> {
    match input
        .get("button")
        .and_then(|v| v.as_str())
        .unwrap_or("left")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "left" => Ok(Button::Left),
        "middle" => Ok(Button::Middle),
        "right" => Ok(Button::Right),
        other => Err(format!("Unsupported mouse button '{}'", other)),
    }
}

fn button_name(button: &Button) -> &'static str {
    match button {
        Button::Left => "left",
        Button::Middle => "middle",
        Button::Right => "right",
        _ => "left",
    }
}

fn parse_key_name(name: &str) -> Result<Key, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Key name cannot be empty".to_string());
    }

    let lower = trimmed.to_ascii_lowercase();
    let key = match lower.as_str() {
        "ctrl" | "control" => Key::Control,
        "shift" => Key::Shift,
        "alt" | "option" => Key::Alt,
        "cmd" | "command" | "meta" | "super" | "win" | "windows" => Key::Meta,
        "enter" | "return" => Key::Return,
        "tab" => Key::Tab,
        "esc" | "escape" => Key::Escape,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "space" => Key::Space,
        "up" | "uparrow" | "arrowup" => Key::UpArrow,
        "down" | "downarrow" | "arrowdown" => Key::DownArrow,
        "left" | "leftarrow" | "arrowleft" => Key::LeftArrow,
        "right" | "rightarrow" | "arrowright" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "insert" => Key::Insert,
        "capslock" => Key::CapsLock,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        _ => {
            let mut chars = trimmed.chars();
            let Some(first) = chars.next() else {
                return Err("Key name cannot be empty".to_string());
            };
            if chars.next().is_none() {
                Key::Unicode(first)
            } else {
                return Err(format!("Unsupported key '{}'", trimmed));
            }
        }
    };

    Ok(key)
}

fn display_json(screen: &Screen) -> Value {
    let info = screen.display_info;
    json!({
        "id": info.id,
        "x": info.x,
        "y": info.y,
        "width": info.width,
        "height": info.height,
        "scale_factor": info.scale_factor,
        "is_primary": info.is_primary,
        "rotation": info.rotation,
        "frequency": info.frequency
    })
}

fn find_screen_by_id(display_id: Option<u32>) -> Result<Screen, String> {
    let screens = Screen::all().map_err(|e| format!("Failed to enumerate displays: {}", e))?;
    if screens.is_empty() {
        return Err("No displays are available for computer_use".to_string());
    }

    match display_id {
        Some(id) => screens
            .into_iter()
            .find(|screen| screen.display_info.id == id)
            .ok_or_else(|| format!("Display '{}' was not found", id)),
        None => screens
            .iter()
            .find(|screen| screen.display_info.is_primary)
            .copied()
            .or_else(|| screens.first().copied())
            .ok_or_else(|| "No displays are available for computer_use".to_string()),
    }
}

fn capture_png_base64(screen: Screen, region: Option<&Value>) -> Result<Value, String> {
    let image = if let Some(region) = region {
        let x = region
            .get("x")
            .and_then(|v| v.as_i64())
            .and_then(|v| i32::try_from(v).ok())
            .ok_or_else(|| "region.x must be an integer".to_string())?;
        let y = region
            .get("y")
            .and_then(|v| v.as_i64())
            .and_then(|v| i32::try_from(v).ok())
            .ok_or_else(|| "region.y must be an integer".to_string())?;
        let width = region
            .get("width")
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| "region.width must be a positive integer".to_string())?;
        let height = region
            .get("height")
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| "region.height must be a positive integer".to_string())?;

        let local_x = x - screen.display_info.x;
        let local_y = y - screen.display_info.y;
        screen
            .capture_area(local_x, local_y, width, height)
            .map_err(|e| format!("Failed to capture screenshot region: {}", e))?
    } else {
        screen
            .capture()
            .map_err(|e| format!("Failed to capture screenshot: {}", e))?
    };

    let width = image.width();
    let height = image.height();
    let mut cursor = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(|e| format!("Failed to encode screenshot as PNG: {}", e))?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(cursor.into_inner());

    Ok(json!({
        "media_type": SCREENSHOT_MEDIA_TYPE,
        "width": width,
        "height": height,
        "data": encoded
    }))
}

fn new_enigo() -> Result<Enigo, String> {
    Enigo::new(&Settings::default())
        .map_err(|e| format!("Failed to initialize desktop input backend: {}", e))
}

fn perform_click(mut enigo: Enigo, x: i32, y: i32, button: Button, count: u64) -> Result<Value, String> {
    enigo
        .move_mouse(x, y, Coordinate::Abs)
        .map_err(|e| format!("Failed to move mouse: {}", e))?;
    for _ in 0..count {
        enigo
            .button(button, Direction::Click)
            .map_err(|e| format!("Failed to click mouse: {}", e))?;
    }
    Ok(json!({
        "ok": true,
        "x": x,
        "y": y,
        "button": button_name(&button),
        "count": count
    }))
}

fn perform_hotkey(mut enigo: Enigo, keys: &[String]) -> Result<Value, String> {
    if keys.len() < 2 {
        return Err("hotkey requires at least two key names".to_string());
    }

    let mut parsed = Vec::with_capacity(keys.len());
    for key in keys {
        parsed.push(parse_key_name(key)?);
    }

    for key in parsed.iter().take(parsed.len() - 1) {
        enigo
            .key(*key, Direction::Press)
            .map_err(|e| format!("Failed to press modifier key: {}", e))?;
    }

    let result = enigo
        .key(*parsed.last().expect("checked len"), Direction::Click)
        .map_err(|e| format!("Failed to send hotkey: {}", e));

    for key in parsed.iter().take(parsed.len() - 1).rev() {
        let _ = enigo.key(*key, Direction::Release);
    }

    result?;

    Ok(json!({
        "ok": true,
        "keys": keys,
        "executed": true
    }))
}

fn execute_blocking(action: String, input: Value) -> Result<Value, String> {
    match action.as_str() {
        "request_access" => Ok(json!({
            "ok": true,
            "action": "request_access",
            "status": "granted",
            "note": "computer_use permission gate passed for this conversation."
        })),
        "list_displays" => {
            let screens = Screen::all()
                .map_err(|e| format!("Failed to enumerate displays: {}", e))?;
            let displays = screens.iter().map(display_json).collect::<Vec<_>>();
            Ok(json!({
                "ok": true,
                "action": "list_displays",
                "displays": displays
            }))
        }
        "cursor_position" => {
            let enigo = new_enigo()?;
            let (x, y) = enigo
                .location()
                .map_err(|e| format!("Failed to get cursor position: {}", e))?;
            let display = Screen::from_point(x, y)
                .ok()
                .map(|screen| display_json(&screen));
            Ok(json!({
                "ok": true,
                "action": "cursor_position",
                "x": x,
                "y": y,
                "display": display
            }))
        }
        "screenshot" => {
            let display_id = input
                .get("display_id")
                .and_then(|v| v.as_u64())
                .and_then(|v| u32::try_from(v).ok());
            let screen = if let Some(region) = input.get("region") {
                let x = region
                    .get("x")
                    .and_then(|v| v.as_i64())
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or_else(|| "region.x must be an integer".to_string())?;
                let y = region
                    .get("y")
                    .and_then(|v| v.as_i64())
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or_else(|| "region.y must be an integer".to_string())?;
                Screen::from_point(x, y)
                    .map_err(|e| format!("Failed to find display for screenshot region: {}", e))?
            } else {
                find_screen_by_id(display_id)?
            };
            let image = capture_png_base64(screen, input.get("region"))?;
            Ok(json!({
                "ok": true,
                "action": "screenshot",
                "display": display_json(&screen),
                "region": input.get("region").cloned(),
                "image": image,
                "note": "Screenshot captured and attached as an additional image context message."
            }))
        }
        "move_mouse" => {
            let x = parse_i32_field(&input, "x")?;
            let y = parse_i32_field(&input, "y")?;
            let mut enigo = new_enigo()?;
            enigo
                .move_mouse(x, y, Coordinate::Abs)
                .map_err(|e| format!("Failed to move mouse: {}", e))?;
            Ok(json!({
                "ok": true,
                "action": "move_mouse",
                "x": x,
                "y": y
            }))
        }
        "click" => {
            let x = parse_i32_field(&input, "x")?;
            let y = parse_i32_field(&input, "y")?;
            let count = input.get("count").and_then(|v| v.as_u64()).unwrap_or(1).clamp(1, 3);
            let button = parse_button(&input)?;
            let enigo = new_enigo()?;
            let mut out = perform_click(enigo, x, y, button, count)?;
            out["action"] = Value::String("click".into());
            Ok(out)
        }
        "double_click" => {
            let x = parse_i32_field(&input, "x")?;
            let y = parse_i32_field(&input, "y")?;
            let button = parse_button(&input)?;
            let enigo = new_enigo()?;
            let mut out = perform_click(enigo, x, y, button, 2)?;
            out["action"] = Value::String("double_click".into());
            Ok(out)
        }
        "drag" => {
            let from_x = parse_i32_field(&input, "from_x")?;
            let from_y = parse_i32_field(&input, "from_y")?;
            let to_x = parse_i32_field(&input, "to_x")?;
            let to_y = parse_i32_field(&input, "to_y")?;
            let mut enigo = new_enigo()?;
            enigo
                .move_mouse(from_x, from_y, Coordinate::Abs)
                .map_err(|e| format!("Failed to move mouse to drag start: {}", e))?;
            enigo
                .button(Button::Left, Direction::Press)
                .map_err(|e| format!("Failed to press mouse button for drag: {}", e))?;
            thread::sleep(Duration::from_millis(DEFAULT_DRAG_SETTLE_MS));
            let move_result = enigo.move_mouse(to_x, to_y, Coordinate::Abs);
            let release_result = enigo.button(Button::Left, Direction::Release);
            move_result.map_err(|e| format!("Failed to drag mouse: {}", e))?;
            release_result.map_err(|e| format!("Failed to release mouse after drag: {}", e))?;
            Ok(json!({
                "ok": true,
                "action": "drag",
                "from": { "x": from_x, "y": from_y },
                "to": { "x": to_x, "y": to_y }
            }))
        }
        "scroll" => {
            let x = parse_i32_field(&input, "x")?;
            let y = parse_i32_field(&input, "y")?;
            let dx = input.get("dx").and_then(|v| v.as_i64()).unwrap_or(0);
            let dy = input.get("dy").and_then(|v| v.as_i64()).unwrap_or(0);
            let mut enigo = new_enigo()?;
            enigo
                .move_mouse(x, y, Coordinate::Abs)
                .map_err(|e| format!("Failed to move mouse before scroll: {}", e))?;
            if dy != 0 {
                enigo
                    .scroll(i32::try_from(dy).map_err(|_| "dy is out of range".to_string())?, Axis::Vertical)
                    .map_err(|e| format!("Failed to scroll vertically: {}", e))?;
            }
            if dx != 0 {
                enigo
                    .scroll(i32::try_from(dx).map_err(|_| "dx is out of range".to_string())?, Axis::Horizontal)
                    .map_err(|e| format!("Failed to scroll horizontally: {}", e))?;
            }
            Ok(json!({
                "ok": true,
                "action": "scroll",
                "x": x,
                "y": y,
                "dx": dx,
                "dy": dy
            }))
        }
        "type_text" => {
            let text = input
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "computer_use type_text requires string 'text'".to_string())?;
            let mut enigo = new_enigo()?;
            enigo
                .text(text)
                .map_err(|e| format!("Failed to type text: {}", e))?;
            Ok(json!({
                "ok": true,
                "action": "type_text",
                "typed_chars": text.chars().count()
            }))
        }
        "key" => {
            let key_name = input
                .get("key_name")
                .or_else(|| input.get("key"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| "computer_use key requires 'key_name' or 'key'".to_string())?;
            let repeat = input.get("count").and_then(|v| v.as_u64()).unwrap_or(1).clamp(1, 20);
            let key = parse_key_name(key_name)?;
            let mut enigo = new_enigo()?;
            for _ in 0..repeat {
                enigo
                    .key(key, Direction::Click)
                    .map_err(|e| format!("Failed to send key: {}", e))?;
            }
            Ok(json!({
                "ok": true,
                "action": "key",
                "key": key_name,
                "repeat": repeat
            }))
        }
        "hotkey" => {
            let keys = input
                .get("keys")
                .and_then(|v| v.as_array())
                .ok_or_else(|| "computer_use hotkey requires string array 'keys'".to_string())?
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            let enigo = new_enigo()?;
            let mut out = perform_hotkey(enigo, &keys)?;
            out["action"] = Value::String("hotkey".into());
            Ok(out)
        }
        other => Err(format!("Unsupported computer_use action '{}'", other)),
    }
}

pub async fn execute_with_app(
    _app: &AppHandle,
    _conversation_id: Option<&str>,
    input: Value,
) -> String {
    let action = input
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    if action == "wait" {
        let wait_ms = parse_u64_field(&input, "duration_ms")
            .or_else(|| parse_u64_field(&input, "ms"))
            .unwrap_or(DEFAULT_WAIT_MS)
            .clamp(1, MAX_WAIT_MS);
        tokio::time::sleep(Duration::from_millis(wait_ms)).await;
        return json!({
            "ok": true,
            "action": "wait",
            "duration_ms": wait_ms
        })
        .to_string();
    }

    let _guard = session_lock().lock().await;
    match tokio::task::spawn_blocking(move || execute_blocking(action, input)).await {
        Ok(Ok(value)) => value.to_string(),
        Ok(Err(err)) => json!({ "ok": false, "error": err }).to_string(),
        Err(err) => json!({
            "ok": false,
            "error": format!("computer_use worker failed: {}", err)
        })
        .to_string(),
    }
}

pub fn postprocess_output(output: &str) -> (String, Vec<Message>) {
    let Ok(mut value) = serde_json::from_str::<Value>(output) else {
        return (output.to_string(), Vec::new());
    };

    let Some(object) = value.as_object_mut() else {
        return (output.to_string(), Vec::new());
    };
    let Some(image_value) = object.get_mut("image") else {
        return (output.to_string(), Vec::new());
    };
    let Some(image_object) = image_value.as_object_mut() else {
        return (output.to_string(), Vec::new());
    };

    let Some(Value::String(data)) = image_object.remove("data") else {
        return (output.to_string(), Vec::new());
    };
    let media_type = image_object
        .get("media_type")
        .and_then(|v| v.as_str())
        .unwrap_or(SCREENSHOT_MEDIA_TYPE)
        .to_string();
    let width = image_object.get("width").and_then(|v| v.as_u64());
    let height = image_object.get("height").and_then(|v| v.as_u64());
    image_object.insert("attached_to_context".to_string(), Value::Bool(true));

    let mut text = "Computer-use screenshot attached. Inspect the image before deciding the next desktop action.".to_string();
    if let (Some(width), Some(height)) = (width, height) {
        text = format!(
            "Computer-use screenshot attached ({}x{}). Inspect the image before deciding the next desktop action.",
            width, height
        );
    }

    let message = Message {
        role: Role::User,
        content: Content::Blocks(vec![
            ContentBlock::Text { text },
            ContentBlock::Image {
                source: ImageSource {
                    source_type: "base64".to_string(),
                    media_type,
                    data,
                },
            },
        ]),
    };

    (
        serde_json::to_string(&value).unwrap_or_else(|_| output.to_string()),
        vec![message],
    )
}
