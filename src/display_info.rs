#[cfg(target_os = "macos")]
use core_graphics::display::CGDisplay;
use scrap::Display;
use crate::error::ScreenRecError;

#[derive(Debug, Clone)]
pub struct DisplayInfo {
    pub index: usize,
    pub width: usize,
    pub height: usize,
    pub x: i32,
    pub y: i32,
    #[allow(dead_code)]
    pub is_primary: bool,
}

/// Get all displays with their bounds information
#[cfg(target_os = "macos")]
pub fn get_all_displays_with_bounds() -> Result<Vec<DisplayInfo>, ScreenRecError> {
    let displays = Display::all().map_err(|e| {
        ScreenRecError::CaptureError(format!("Failed to enumerate displays: {}", e))
    })?;

    let mut display_infos = Vec::new();

    for (index, display) in displays.iter().enumerate() {
        // Get CGDisplay for this display index
        let cg_display = CGDisplay::new(index as u32);
        let bounds = cg_display.bounds();

        display_infos.push(DisplayInfo {
            index,
            width: display.width(),
            height: display.height(),
            x: bounds.origin.x as i32,
            y: bounds.origin.y as i32,
            is_primary: index == 0, // Primary display is typically index 0
        });
    }

    Ok(display_infos)
}

/// Get all displays with their bounds information (Windows implementation)
#[cfg(target_os = "windows")]
pub fn get_all_displays_with_bounds() -> Result<Vec<DisplayInfo>, ScreenRecError> {
    let displays = Display::all().map_err(|e| {
        ScreenRecError::CaptureError(format!("Failed to enumerate displays: {}", e))
    })?;

    let mut display_infos = Vec::new();

    for (index, display) in displays.iter().enumerate() {
        // On Windows, scrap doesn't provide position info, so we default to (0, 0)
        // This is a limitation but shouldn't affect single-display recording
        display_infos.push(DisplayInfo {
            index,
            width: display.width(),
            height: display.height(),
            x: 0,
            y: 0,
            is_primary: index == 0, // Primary display is typically index 0
        });
    }

    Ok(display_infos)
}

/// Determine which display contains the given cursor position (macOS)
#[cfg(target_os = "macos")]
pub fn get_display_at_cursor(cursor_x: i32, cursor_y: i32) -> Result<usize, ScreenRecError> {
    let displays = get_all_displays_with_bounds()?;

    // Find which display contains the cursor
    for display in &displays {
        let x_min = display.x;
        let x_max = display.x + display.width as i32;
        let y_min = display.y;
        let y_max = display.y + display.height as i32;

        if cursor_x >= x_min && cursor_x < x_max && cursor_y >= y_min && cursor_y < y_max {
            return Ok(display.index);
        }
    }

    // If cursor is not on any display (shouldn't happen), return primary
    Ok(0)
}

/// Determine which display contains the given cursor position (Windows)
/// Note: Windows implementation returns primary display as we don't have
/// accurate position information without additional Windows API calls
#[cfg(target_os = "windows")]
pub fn get_display_at_cursor(_cursor_x: i32, _cursor_y: i32) -> Result<usize, ScreenRecError> {
    // On Windows, without proper display position info, we default to primary display
    // This is a simplified implementation that works for single-monitor setups
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_all_displays_with_bounds() {
        // This test will only pass if displays are available
        if let Ok(displays) = get_all_displays_with_bounds() {
            assert!(!displays.is_empty());
            assert!(displays[0].is_primary);
        }
    }
}
