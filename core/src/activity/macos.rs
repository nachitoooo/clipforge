//! The application in front, from `NSWorkspace`. Needs no permission: it is
//! the same list the Dock and ⌘-Tab show.

use objc2_app_kit::NSWorkspace;

use super::{AppInfo, ForegroundSource};

pub struct MacForeground;

impl ForegroundSource for MacForeground {
    fn foreground(&mut self) -> Option<AppInfo> {
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        let bundle = app.bundleIdentifier().map(|id| id.to_string());
        let name = app
            .localizedName()
            .map(|name| name.to_string())
            .or_else(|| bundle.clone())?;
        // A program that isn't an app bundle (a development build) has no
        // bundle id: its name has to serve as its identity.
        let id = bundle.unwrap_or_else(|| name.to_lowercase());
        Some(AppInfo { id, name })
    }
}
