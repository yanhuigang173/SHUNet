use serde::{Deserialize, Serialize};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MenuTheme {
    #[default]
    Default,
    Classic,
    Modern,
}
impl MenuTheme {
    pub const ALL: [Self; 3] = [Self::Default, Self::Classic, Self::Modern];
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "默认",
            Self::Classic => "经典",
            Self::Modern => "现代",
        }
    }
    pub fn background(self) -> u32 {
        match self {
            Self::Default => rgb(249, 249, 249),
            Self::Modern => rgb(255, 255, 255),
            Self::Classic => unsafe {
                windows_sys::Win32::Graphics::Gdi::GetSysColor(
                    windows_sys::Win32::Graphics::Gdi::COLOR_MENU,
                )
            },
        }
    }
}
pub const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
thread_local! { static BUILD_THEME:Cell<MenuTheme> = const {Cell::new(MenuTheme::Default)}; }
pub fn current() -> MenuTheme {
    BUILD_THEME.with(Cell::get)
}
/// Freeze a theme for the complete popup tree; never restyle a visible menu.
pub struct Session(MenuTheme);
impl Session {
    pub fn new(theme: MenuTheme) -> Self {
        Self(BUILD_THEME.with(|slot| slot.replace(theme)))
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        BUILD_THEME.with(|slot| slot.set(self.0));
    }
}
