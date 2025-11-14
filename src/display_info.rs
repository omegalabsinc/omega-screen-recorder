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

/// Determine which display contains the given cursor position
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
