//! Shared native dialog design system. Resources exist only while a dialog is open.
use std::cell::RefCell;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::DwmSetWindowAttribute, Gdi::*},
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::*, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};

const fn color(r: u32, g: u32, b: u32) -> u32 {
    r | g << 8 | b << 16
}
pub const CANVAS: u32 = color(245, 247, 251);
pub const SURFACE: u32 = color(255, 255, 255);
pub const TEXT: u32 = color(25, 43, 68);
pub const MUTED: u32 = color(96, 113, 138);
pub const BORDER: u32 = color(223, 230, 240);
pub const BLUE: u32 = color(37, 99, 235);
const BLUE_SOFT: u32 = color(234, 241, 255);
const TEAL: u32 = color(9, 142, 123);
pub const GREEN: u32 = color(22, 139, 93);
const GREEN_SOFT: u32 = color(232, 247, 239);
pub const AMBER: u32 = color(166, 107, 13);
const AMBER_SOFT: u32 = color(255, 246, 224);
const SUBCLASS: usize = 0x534855;
const PROPERTY: &[u16] = &[
    83, 72, 85, 78, 101, 116, 46, 85, 105, 84, 104, 101, 109, 101, 0,
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Speed,
    Website,
    Hotkeys,
    Help,
    IpInfo,
    Devices,
    Status,
    Power,
    PowerRun,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Working,
    Success,
    Warning,
}
struct Theme {
    kind: Kind,
    dpi: u32,
    w: i32,
    h: i32,
    fonts: [HFONT; 5],
    brushes: [HBRUSH; 5],
    icon: HICON,
    hot: HWND,
    progress: f32,
    tone: Tone,
    proxy: bool,
}
type State = RefCell<Theme>;
fn pointer(window: HWND) -> *mut State {
    unsafe { GetPropW(window, PROPERTY.as_ptr()).cast() }
}
fn font(size: i32, weight: i32, dpi: u32, family: &str) -> HFONT {
    let mut data = LOGFONTW {
        lfHeight: -(size * dpi as i32 / 96),
        lfWeight: weight,
        lfQuality: CLEARTYPE_QUALITY,
        ..Default::default()
    };
    for (slot, code) in data.lfFaceName.iter_mut().zip(crate::wide(family)) {
        *slot = code;
    }
    unsafe { CreateFontIndirectW(&data) }
}
impl Theme {
    fn new(kind: Kind, dpi: u32) -> Self {
        Self {
            kind,
            dpi,
            w: 720,
            h: 520,
            fonts: [
                font(13, 400, dpi, "Microsoft YaHei UI"),
                font(12, 400, dpi, "Microsoft YaHei UI"),
                font(22, 600, dpi, "Microsoft YaHei UI"),
                font(32, 600, dpi, "Segoe UI"),
                font(13, 600, dpi, "Microsoft YaHei UI"),
            ],
            brushes: [CANVAS, SURFACE, BLUE_SOFT, GREEN_SOFT, AMBER_SOFT]
                .map(|c| unsafe { CreateSolidBrush(c) }),
            icon: unsafe {
                LoadImageW(
                    GetModuleHandleW(std::ptr::null()),
                    1usize as _,
                    IMAGE_ICON,
                    (28 * dpi / 96) as i32,
                    (28 * dpi / 96) as i32,
                    0,
                ) as HICON
            },
            hot: std::ptr::null_mut(),
            progress: 0.0,
            tone: Tone::Working,
            proxy: false,
        }
    }
    fn p(&self, n: i32) -> i32 {
        (n * self.dpi as i32 + 48) / 96
    }
    fn rect(&self, x: i32, y: i32, w: i32, h: i32) -> RECT {
        RECT {
            left: self.p(x),
            top: self.p(y),
            right: self.p(x + w),
            bottom: self.p(y + h),
        }
    }
    fn place(&self, window: HWND, id: i32, x: i32, y: i32, w: i32, h: i32) {
        unsafe {
            let child = GetDlgItem(window, id);
            if !child.is_null() {
                SetWindowPos(
                    child,
                    std::ptr::null_mut(),
                    self.p(x),
                    self.p(y),
                    self.p(w),
                    self.p(h),
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }
    fn layout(&mut self, window: HWND, fit: bool) {
        let (width, height) = match self.kind {
            Kind::Speed => (720, 520),
            Kind::Website => (520, 332),
            Kind::Hotkeys => (720, 548),
            Kind::Help => (720, 548),
            Kind::IpInfo => (800, 620),
            Kind::Devices => (900, 610),
            Kind::Status => (900, 700),
            Kind::Power => (840, 660),
            Kind::PowerRun => (520, 290),
        };
        unsafe {
            if fit {
                let mut area = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                GetMonitorInfoW(
                    MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST),
                    &mut area,
                );
                let mut outer = self.rect(0, 0, width, height);
                AdjustWindowRectExForDpi(
                    &mut outer,
                    GetWindowLongPtrW(window, GWL_STYLE) as u32,
                    0,
                    GetWindowLongPtrW(window, GWL_EXSTYLE) as u32,
                    self.dpi,
                );
                let max_w = (area.rcWork.right - area.rcWork.left - 32).max(320);
                let max_h = (area.rcWork.bottom - area.rcWork.top - 32).max(280);
                SetWindowPos(
                    window,
                    std::ptr::null_mut(),
                    0,
                    0,
                    (outer.right - outer.left).min(max_w),
                    (outer.bottom - outer.top).min(max_h),
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            let mut client = RECT::default();
            GetClientRect(window, &mut client);
            self.w = client.right * 96 / self.dpi as i32;
            self.h = client.bottom * 96 / self.dpi as i32;
        }
        let w = self.w;
        let h = self.h;
        self.place(window, 1600, 84, 21, w - 108, 30);
        self.place(window, 1601, 84, 55, w - 108, 20);
        match self.kind {
            Kind::Speed => {
                self.place(window, 1600, 84, 20, (w - 356).min(220), 30);
                self.place(window, 1601, 84, 54, (w - 272).min(320), 20);
                self.place(window, 1411, w - 248, 22, 108, 32);
                self.place(window, 1412, w - 132, 22, 108, 32);
                self.place(window, 1401, w - 156, 62, 124, 18);
                self.place(window, 1402, 24, 88, w - 48, 26);
                let compact = h < 500;
                let top = if h < 370 {
                    88
                } else if compact {
                    120
                } else {
                    128
                };
                unsafe {
                    ShowWindow(
                        GetDlgItem(window, 1402),
                        if h < 370 { SW_HIDE } else { SW_SHOW },
                    );
                }
                let card_w = (w - 72) / 3;
                for (i, title, value, unit) in [
                    (0, 1417, 1405, 1420),
                    (1, 1415, 1406, 1421),
                    (2, 1416, 1407, 1422),
                ] {
                    let x = 24 + i * (card_w + 12);
                    self.place(window, title, x + 16, top + 14, card_w - 32, 18);
                    self.place(window, value, x + 16, top + 37, card_w - 32, 42);
                    self.place(window, unit, x + 16, top + 81, card_w - 32, 16);
                }
                let stage = if h < 370 {
                    200
                } else if compact {
                    234
                } else {
                    252
                };
                self.place(window, 1404, 24, stage, w - 116, 22);
                self.place(window, 1426, w - 84, stage, 60, 22);
                let history = if compact { 282 } else { 306 };
                self.place(window, 1427, 24, history, 220, 20);
                self.place(window, 1428, w - 244, history, 220, 20);
                let row = history + 40;
                for (base, y) in [(1430, row), (1434, row + 32)] {
                    self.place(window, base, 40, y, 86, 20);
                    self.place(window, base + 1, 148, y, (w - 268) / 3, 20);
                    self.place(window, base + 2, 148 + (w - 268) / 3, y, (w - 268) / 3, 20);
                    self.place(window, base + 3, w - 200, y, 160, 20);
                }
                // Keep the actions accessible on short, highly scaled displays.
                unsafe {
                    for id in [1427, 1428, 1430, 1431, 1432, 1433, 1434, 1435, 1436, 1437] {
                        ShowWindow(
                            GetDlgItem(window, id),
                            if h < 470 { SW_HIDE } else { SW_SHOW },
                        );
                    }
                }
                self.place(window, 1408, 24, h - 98, w - 48, 18);
                unsafe {
                    ShowWindow(
                        GetDlgItem(window, 1408),
                        if compact { SW_HIDE } else { SW_SHOW },
                    );
                }
                let footer_y = if h < 370 { h - 46 } else { h - 58 };
                self.place(window, 1409, 24, footer_y, 110, 34);
                self.place(window, 1413, w - 236, footer_y, 100, 34);
                self.place(window, IDCANCEL, w - 124, footer_y, 100, 34);
            }
            Kind::Website => {
                self.place(window, 1003, 24, 92, w - 48, 18);
                self.place(window, 1001, 36, 120, w - 72, 20);
                self.place(window, 1004, 24, 162, w - 48, 18);
                self.place(window, 1002, 36, 190, w - 72, 20);
                self.place(window, 1005, 24, 232, w - 48, 18);
                unsafe {
                    ShowWindow(
                        GetDlgItem(window, 1005),
                        if h < 328 { SW_HIDE } else { SW_SHOW },
                    );
                }
                self.place(window, IDOK, w - 124, h - 58, 100, 34);
                self.place(window, IDCANCEL, w - 236, h - 58, 100, 34);
            }
            Kind::Help => {
                self.place(window, 1101, 38, 106, w - 76, h - 204);
                self.place(window, 1102, 24, h - 58, 120, 34);
                self.place(window, IDOK, w - 124, h - 58, 100, 34);
            }
            Kind::IpInfo => {
                self.place(window, 1701, 36, 103, 112, 22);
                self.place(window, 1702, 156, 98, w - 192, 240);
                let row = ((h - 278) / 8).clamp(20, 34);
                for index in 0..8 {
                    let y = 138 + index * row;
                    self.place(window, 1730 + index, 36, y + 3, 114, row - 2);
                    self.place(window, 1710 + index, 156, y + 3, w - 280, row - 4);
                    self.place(window, 1750 + index, w - 104, y, 68, row - 2);
                }
                self.place(window, 1704, 36, 146 + row * 8, w - 72, (h - 256 - row * 8).max(18));
                self.place(window, 1703, 36, h - 104, w - 72, 22);
                self.place(window, 1705, 24, h - 58, 108, 34);
                self.place(window, 1706, 144, h - 58, 108, 34);
                self.place(window, IDCANCEL, w - 124, h - 58, 100, 34);
            }
            Kind::Devices => {
                self.place(window, 1701, 36, 103, 82, 22);
                self.place(window, 1702, 122, 98, w - 158, 240);
                self.place(window, 1720, 36, 140, w - 72, (h - 318).max(70));
                self.place(window, 1704, 36, h - 168, w - 72, 58);
                self.place(window, 1703, 36, h - 102, w - 72, 24);
                self.place(window, 1705, 24, h - 58, 108, 34);
                self.place(window, 1706, 144, h - 58, 108, 34);
                self.place(window, 1707, 264, h - 58, 136, 34);
                self.place(window, IDCANCEL, w - 124, h - 58, 100, 34);
            }
            Kind::Power => {
                let cw=(w-88)/2;
                for (id,x,y,ww,hh) in [
                    (1931,36,104,80,22),(1901,120,98,cw-84,220),
                    (1932,52+cw,104,80,22),(1902,136+cw,98,cw-84,160),
                    (1933,36,150,80,22),(1903,120,144,94,28),
                    (1904,226,144,154,28),(1905,392,144,112,28),
                    (1934,36,194,80,22),(1906,120,188,230,180),
                    (1907,370,188,w-406,28),(1908,36,232,w-236,44),
                    (IDOK,w-180,230,144,36),(1909,36,280,w-72,44),
                    (1935,24,332,150,22),(1920,36,366,w-72,(h-534).max(60)),
                    (1914,36,h-158,w-72,62),(1915,24,h-92,w-48,18),
                    (1912,24,h-52,100,32),(1913,132,h-52,80,32),
                    (1911,220,h-52,80,32),(1916,308,h-52,104,32),
                    (1917,420,h-52,108,32),(IDCANCEL,w-124,h-52,100,32)
                ] {self.place(window,id,x,y,ww,hh);}
            }
            Kind::PowerRun => {
                self.place(window,1941,36,110,w-72,44);
                self.place(window,1942,36,156,w-72,44);
                self.place(window,IDCANCEL,w-180,h-54,144,34);
            }
            Kind::Status => {
                let cw = (w - 72) / 3;
                for (i,base) in [(0,1810),(1,1820),(2,1830)] {
                    let x = 24+i*(cw+12);
                    self.place(window,base,x+16,108,cw-32,18);
                    self.place(window,base+1,x+16,133,cw-32,36);
                    self.place(window,base+2,x+16,176,cw-32,18);
                }
                for (id,y) in [(1840,220),(1841,244),(1842,268)] { self.place(window,id,36,y,w-72,20); }
                self.place(window,1850,24,307,96,22);
                self.place(window,1851,124,300,132,220);
                self.place(window,1852,268,300,144,220);
                self.place(window,1853,w-292,300,84,28);
                self.place(window,1855,w-204,305,88,20);
                self.place(window,1854,w-112,300,88,28);
                self.place(window,1860,36,346,w-72,(h-462).max(48));
                self.place(window,1861,24,h-101,w-48,20);
                for (i,id) in [1870,1871,1872,1873,1874].into_iter().enumerate() {
                    self.place(window,id,24+i as i32*114,h-58,104,34);
                }
                self.place(window,IDCANCEL,w-124,h-58,100,34);
            }
            Kind::Hotkeys => {
                let list_w = (w * 58 / 100).max(280);
                self.place(window, 1300, 24, 89, w - 48, 32);
                self.place(window, 1301, 32, 136, list_w - 16, h - 222);
                self.place(
                    window,
                    1302,
                    list_w + 48,
                    138,
                    w - list_w - 72,
                    if h < 370 { 24 } else { 44 },
                );
                self.place(
                    window,
                    1303,
                    list_w + 60,
                    if h < 370 { 188 } else { 202 },
                    w - list_w - 96,
                    24,
                );
                if h < 370 {
                    self.place(window, 1304, 148, h - 58, 110, 34);
                } else {
                    self.place(window, 1304, list_w + 48, 250, 110, 32);
                }
                self.place(
                    window,
                    1305,
                    list_w + 48,
                    300,
                    w - list_w - 72,
                    (h - 382).max(18),
                );
                unsafe {
                    ShowWindow(
                        GetDlgItem(window, 1305),
                        if h < 440 { SW_HIDE } else { SW_SHOW },
                    );
                }
                self.place(window, 1306, 24, h - 58, 110, 34);
                self.place(window, IDOK, w - 124, h - 58, 100, 34);
                self.place(window, IDCANCEL, w - 236, h - 58, 100, 34);
                unsafe {
                    SendDlgItemMessageW(window, 1301, LB_SETITEMHEIGHT, 0, self.p(32) as isize);
                }
            }
        }
        unsafe {
            EnumChildWindows(window, Some(style_child), self as *const Self as isize);
            InvalidateRect(window, std::ptr::null(), 0);
        }
    }
    fn font_for(&self, id: i32) -> HFONT {
        if id == 1600 {
            self.fonts[2]
        } else if self.kind==Kind::PowerRun && id==1941 {self.fonts[2]
        } else if self.kind==Kind::Power && id==1915 {self.fonts[1]
        } else if (self.kind == Kind::Speed && [1405, 1406, 1407].contains(&id)) || (self.kind == Kind::Status && id == 1821) {
            self.fonts[3]
        } else if [1415, 1416, 1417, 1427, 1302, 1811, 1831].contains(&id) {
            self.fonts[4]
        } else if [
            1601, 1401, 1402, 1408, 1420, 1421, 1422, 1423, 1424, 1425, 1426, 1428, 1433, 1437,
            1005, 1300, 1305, 1703, 1704,
        ]
        .contains(&id)
        {
            self.fonts[1]
        } else {
            self.fonts[0]
        }
    }
    fn text_color(&self, id: i32) -> u32 {
        if [1908,1941].contains(&id) {return BLUE;}
        if [1909,1914,1915,1942].contains(&id) {return MUTED;}
        match id {
            1406 | 1821 => BLUE,
            1407 => TEAL,
            1426 => BLUE,
            1401 | 1811 => match self.tone {
                Tone::Working => BLUE,
                Tone::Success => GREEN,
                Tone::Warning => AMBER,
            },
            1810 | 1812 | 1820 | 1822 | 1830 | 1832 | 1841 | 1861 | 1601 | 1402 | 1408 | 1415 | 1416 | 1417 | 1420 | 1421 | 1422 | 1423 | 1424 | 1425
            | 1428 | 1433 | 1437 | 1005 | 1300 | 1305 => MUTED,
            _ => TEXT,
        }
    }
    fn background(&self, id: i32) -> (u32, HBRUSH) {
        if self.kind == Kind::Speed && id == 1401 {
            return match self.tone {
                Tone::Working => (BLUE_SOFT, self.brushes[2]),
                Tone::Success => (GREEN_SOFT, self.brushes[3]),
                Tone::Warning => (AMBER_SOFT, self.brushes[4]),
            };
        }
        let surface = (self.kind == Kind::Power && ((1901..=1909).contains(&id) || (1931..=1934).contains(&id) || id==1914 || id==1920)) || (self.kind==Kind::PowerRun && [1941,1942].contains(&id)) || (self.kind == Kind::Status && ((1810..=1832).contains(&id) || id == 1860)) || [
            1600, 1601, 1402, 1405, 1406, 1407, 1415, 1416, 1417, 1420, 1421, 1422, 1423, 1424,
            1425, 1430, 1431, 1432, 1433, 1434, 1435, 1436, 1437, 1001, 1002, 1101, 1301, 1302,
            1303, 1305,
        ]
        .contains(&id);
        if surface {
            (SURFACE, self.brushes[1])
        } else {
            (CANVAS, self.brushes[0])
        }
    }
    fn paint(&self, window: HWND) {
        unsafe {
            let mut paint = PAINTSTRUCT::default();
            let dc = BeginPaint(window, &mut paint);
            FillRect(dc, &paint.rcPaint, self.brushes[0]);
            FillRect(
                dc,
                &self.rect(
                    0,
                    0,
                    self.w,
                    if self.kind == Kind::Speed && self.h >= 370 {
                        116
                    } else {
                        84
                    },
                ),
                self.brushes[1],
            );
            let footer = if self.kind == Kind::Speed && self.h < 370 {
                58
            } else {
                74
            };
            FillRect(
                dc,
                &self.rect(0, self.h - footer, self.w, footer),
                self.brushes[1],
            );
            FillRect(
                dc,
                &self.rect(0, self.h - footer, self.w, 1),
                brush(BORDER).0,
            );
            rounded(
                dc,
                self.rect(24, 20, 44, 44),
                BLUE_SOFT,
                BLUE_SOFT,
                self.p(12),
            );
            if !self.icon.is_null() {
                DrawIconEx(
                    dc,
                    self.p(32),
                    self.p(28),
                    self.icon,
                    self.p(28),
                    self.p(28),
                    0,
                    std::ptr::null_mut(),
                    DI_NORMAL,
                );
            }
            match self.kind {
                Kind::Speed => {
                    let compact = self.h < 500;
                    let top = if self.h < 370 {
                        88
                    } else if compact {
                        120
                    } else {
                        128
                    };
                    let cw = (self.w - 72) / 3;
                    for i in 0..3 {
                        rounded(
                            dc,
                            self.rect(24 + i * (cw + 12), top, cw, if compact { 100 } else { 110 }),
                            SURFACE,
                            BORDER,
                            self.p(12),
                        );
                    }
                    let y = if self.h < 370 {
                        228
                    } else if compact {
                        262
                    } else {
                        280
                    };
                    rounded(
                        dc,
                        self.rect(24, y, self.w - 48, 6),
                        BORDER,
                        BORDER,
                        self.p(3),
                    );
                    let filled = ((self.w - 48) as f32 * self.progress.clamp(0.0, 1.0)) as i32;
                    if filled > 0 {
                        let c = match self.tone {
                            Tone::Success => GREEN,
                            Tone::Warning => AMBER,
                            Tone::Working => BLUE,
                        };
                        rounded(dc, self.rect(24, y, filled, 6), c, c, self.p(3));
                    }
                    if self.h >= 470 {
                        let hist = if compact { 312 } else { 336 };
                        rounded(
                            dc,
                            self.rect(24, hist, self.w - 48, 72),
                            SURFACE,
                            BORDER,
                            self.p(10),
                        );
                        let line = brush(BORDER);
                        FillRect(dc, &self.rect(40, hist + 36, self.w - 80, 1), line.0);
                    }
                    let c = match self.tone {
                        Tone::Working => BLUE_SOFT,
                        Tone::Success => GREEN_SOFT,
                        Tone::Warning => AMBER_SOFT,
                    };
                    rounded(dc, self.rect(self.w - 164, 58, 140, 26), c, c, self.p(13));
                }
                Kind::Website => {
                    for y in [112, 182] {
                        let focused =
                            GetFocus() == GetDlgItem(window, if y == 112 { 1001 } else { 1002 });
                        rounded(
                            dc,
                            self.rect(24, y, self.w - 48, 36),
                            SURFACE,
                            if focused { BLUE } else { BORDER },
                            self.p(8),
                        );
                    }
                }
                Kind::Status => {
                    let cw=(self.w-72)/3;
                    for i in 0..3 { rounded(dc,self.rect(24+i*(cw+12),98,cw,106),SURFACE,BORDER,self.p(12)); }
                    rounded(dc,self.rect(24,336,self.w-48,(self.h-442).max(68)),SURFACE,BORDER,self.p(12));
                }
                Kind::Power => {
                    rounded(dc,self.rect(24,92,self.w-48,234),SURFACE,BORDER,self.p(12));
                    rounded(dc,self.rect(118,142,98,32),SURFACE,BORDER,self.p(6));
                    rounded(dc,self.rect(24,356,self.w-48,(self.h-450).max(80)),SURFACE,BORDER,self.p(12));
                }
                Kind::Help | Kind::IpInfo | Kind::Devices | Kind::PowerRun => rounded(
                    dc,
                    self.rect(24, 92, self.w - 48, self.h - 176),
                    SURFACE,
                    BORDER,
                    self.p(12),
                ),
                Kind::Hotkeys => {
                    let lw = (self.w * 58 / 100).max(280);
                    rounded(
                        dc,
                        self.rect(24, 128, lw, self.h - 206),
                        SURFACE,
                        BORDER,
                        self.p(12),
                    );
                    rounded(
                        dc,
                        self.rect(lw + 32, 128, self.w - lw - 56, self.h - 206),
                        SURFACE,
                        BORDER,
                        self.p(12),
                    );
                    rounded(
                        dc,
                        self.rect(
                            lw + 48,
                            if self.h < 370 { 176 } else { 190 },
                            self.w - lw - 72,
                            if self.h < 370 { 40 } else { 48 },
                        ),
                        SURFACE,
                        if GetFocus() == GetDlgItem(window, 1303) {
                            BLUE
                        } else {
                            BORDER
                        },
                        self.p(8),
                    );
                }
            }
            EndPaint(window, &paint);
        }
    }
}
impl Drop for Theme {
    fn drop(&mut self) {
        unsafe {
            for font in self.fonts {
                DeleteObject(font as _);
            }
            for brush in self.brushes {
                DeleteObject(brush as _);
            }
            if !self.icon.is_null() {
                DestroyIcon(self.icon);
            }
        }
    }
}
struct Brush(HBRUSH);
impl Drop for Brush {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.0 as _);
        }
    }
}
fn brush(c: u32) -> Brush {
    Brush(unsafe { CreateSolidBrush(c) })
}
fn rounded(dc: HDC, r: RECT, fill: u32, border: u32, radius: i32) {
    unsafe {
        let b = brush(fill);
        let pen = CreatePen(PS_SOLID, 1, border);
        let old_b = SelectObject(dc, b.0 as _);
        let old_p = SelectObject(dc, pen as _);
        RoundRect(dc, r.left, r.top, r.right, r.bottom, radius * 2, radius * 2);
        SelectObject(dc, old_p);
        SelectObject(dc, old_b);
        DeleteObject(pen as _);
    }
}
unsafe extern "system" fn style_child(child: HWND, data: LPARAM) -> i32 {
    unsafe {
        let theme = &*(data as *const Theme);
        let id = GetDlgCtrlID(child);
        SendMessageW(child, WM_SETFONT, theme.font_for(id) as usize, 0);
        let mut class = [0u16; 32];
        let n = GetClassNameW(child, class.as_mut_ptr(), 32);
        let class = String::from_utf16_lossy(&class[..n.max(0) as usize]);
        if class.eq_ignore_ascii_case("Button") {
            let style = GetWindowLongPtrW(child, GWL_STYLE) as u32;
            let button_type=style & BS_TYPEMASK as u32;
            if [BS_CHECKBOX,BS_AUTOCHECKBOX,BS_3STATE,BS_AUTO3STATE,BS_RADIOBUTTON,BS_AUTORADIOBUTTON].iter().any(|kind|*kind as u32==button_type) {
                SetWindowTheme(child,crate::wide("Explorer").as_ptr(),std::ptr::null());
            } else if button_type != BS_GROUPBOX as u32 {
                SetWindowLongPtrW(
                    child,
                    GWL_STYLE,
                    ((style & !(BS_TYPEMASK as u32)) | BS_OWNERDRAW as u32) as isize,
                );
                SetWindowSubclass(child, Some(child_proc), SUBCLASS, 0);
            }
        } else if class.eq_ignore_ascii_case("Edit") || class.eq_ignore_ascii_case("ListBox") {
            SetWindowLongPtrW(
                child,
                GWL_EXSTYLE,
                GetWindowLongPtrW(child, GWL_EXSTYLE) & !(WS_EX_CLIENTEDGE as isize),
            );
            SetWindowLongPtrW(
                child,
                GWL_STYLE,
                GetWindowLongPtrW(child, GWL_STYLE) & !(WS_BORDER as isize),
            );
            SetWindowTheme(child, crate::wide("Explorer").as_ptr(), std::ptr::null());
            SetWindowPos(
                child,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            SetWindowSubclass(child, Some(child_proc), SUBCLASS, 0);
        }
    }
    1
}
unsafe extern "system" fn child_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    let parent = unsafe { GetParent(window) };
    let state = pointer(parent);
    if !state.is_null() {
        if let Ok(mut theme) = unsafe { &*state }.try_borrow_mut() {
            if message == WM_MOUSEMOVE && theme.hot != window {
                let old = theme.hot;
                theme.hot = window;
                unsafe {
                    InvalidateRect(window, std::ptr::null(), 0);
                    if !old.is_null() {
                        InvalidateRect(old, std::ptr::null(), 0);
                    }
                    let mut event = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: window,
                        dwHoverTime: 0,
                    };
                    TrackMouseEvent(&mut event);
                }
            } else if message == WM_MOUSELEAVE {
                theme.hot = std::ptr::null_mut();
                unsafe {
                    InvalidateRect(window, std::ptr::null(), 0);
                }
            } else if message == WM_SETFOCUS || message == WM_KILLFOCUS {
                unsafe {
                    InvalidateRect(parent, std::ptr::null(), 0);
                }
            }
        }
    }
    if message == WM_NCDESTROY {
        unsafe {
            RemoveWindowSubclass(window, Some(child_proc), SUBCLASS);
        }
    }
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}
fn draw_item(theme: &Theme, draw: &DRAWITEMSTRUCT) {
    unsafe {
        let dc = draw.hDC;
        SetBkMode(dc, TRANSPARENT as i32);
        if draw.CtlType == ODT_LISTBOX {
            let selected = draw.itemState & ODS_SELECTED != 0;
            let bg = brush(if selected { BLUE_SOFT } else { SURFACE });
            FillRect(dc, &draw.rcItem, bg.0);
            if draw.itemID == u32::MAX {
                return;
            }
            let len = SendMessageW(draw.hwndItem, LB_GETTEXTLEN, draw.itemID as usize, 0);
            if !(0..=1024).contains(&len) {
                return;
            }
            let mut text = vec![0u16; len as usize + 1];
            SendMessageW(
                draw.hwndItem,
                LB_GETTEXT,
                draw.itemID as usize,
                text.as_mut_ptr() as isize,
            );
            let text = String::from_utf16_lossy(&text[..len as usize]);
            let (name, key) = text.split_once('\t').unwrap_or((&text, ""));
            let mut left = draw.rcItem;
            left.left += theme.p(12);
            left.right -= theme.p(130);
            let mut right = draw.rcItem;
            right.left = right.right - theme.p(128);
            right.right -= theme.p(12);
            let old = SelectObject(dc, theme.fonts[0] as _);
            SetTextColor(dc, if selected { BLUE } else { TEXT });
            DrawTextW(
                dc,
                crate::wide(name).as_ptr(),
                -1,
                &mut left,
                DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            SelectObject(dc, theme.fonts[4] as _);
            SetTextColor(dc, if key == "未设置" { MUTED } else { BLUE });
            DrawTextW(
                dc,
                crate::wide(key).as_ptr(),
                -1,
                &mut right,
                DT_SINGLELINE | DT_VCENTER | DT_RIGHT | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            if draw.itemState & ODS_FOCUS != 0 {
                let mut focus = draw.rcItem;
                focus.left += 2;
                focus.right -= 2;
                DrawFocusRect(dc, &focus);
            }
            SelectObject(dc, old);
            return;
        }
        let id = draw.CtlID as i32;
        let enabled = draw.itemState & ODS_DISABLED == 0;
        let primary = id == IDOK || (theme.kind == Kind::Speed && id == IDCANCEL);
        let segment = theme.kind == Kind::Speed && [1411, 1412].contains(&id);
        let active = segment && ((id == 1412) == theme.proxy);
        let hovered = theme.hot == draw.hwndItem;
        let down = draw.itemState & ODS_SELECTED != 0;
        let (bg, edge, fg) = if !enabled && !active {
            (CANVAS, BORDER, color(164, 175, 190))
        } else if primary {
            (
                if down {
                    color(29, 78, 216)
                } else if hovered {
                    color(55, 113, 245)
                } else {
                    BLUE
                },
                BLUE,
                SURFACE,
            )
        } else if active {
            (BLUE_SOFT, BLUE_SOFT, BLUE)
        } else if id == 1409 && theme.tone == Tone::Working {
            (
                color(255, 241, 241),
                color(251, 215, 215),
                color(190, 58, 58),
            )
        } else {
            (
                if hovered { BLUE_SOFT } else { SURFACE },
                if hovered { BLUE } else { BORDER },
                TEXT,
            )
        };
        let base = brush(SURFACE);
        FillRect(dc, &draw.rcItem, base.0);
        let mut r = draw.rcItem;
        r.left += 1;
        r.top += 1;
        r.right -= 1;
        r.bottom -= 1;
        rounded(dc, r, bg, edge, theme.p(7));
        let mut text = [0u16; 160];
        GetWindowTextW(draw.hwndItem, text.as_mut_ptr(), 160);
        let old = SelectObject(dc, theme.fonts[4] as _);
        SetTextColor(dc, fg);
        DrawTextW(
            dc,
            text.as_ptr(),
            -1,
            &mut r,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        if draw.itemState & ODS_FOCUS != 0 {
            r.left += theme.p(4);
            r.right -= theme.p(4);
            r.top += theme.p(4);
            r.bottom -= theme.p(4);
            DrawFocusRect(dc, &r);
        }
        SelectObject(dc, old);
    }
}

pub fn init(window: HWND, kind: Kind) {
    crate::dialogs::center_on_cursor(window);
    let state = Box::new(RefCell::new(Theme::new(
        kind,
        unsafe { GetDpiForWindow(window) }.max(96),
    )));
    let pointer = Box::into_raw(state);
    unsafe {
        if SetPropW(window, PROPERTY.as_ptr(), pointer as _) == 0 {
            drop(Box::from_raw(pointer));
            return;
        }
    }
    unsafe {
        SetWindowLongPtrW(
            window,
            GWL_STYLE,
            GetWindowLongPtrW(window, GWL_STYLE) | WS_CLIPCHILDREN as isize,
        );
        let behavior = DDC_DISABLE_RESIZE | DDC_DISABLE_CONTROL_RELAYOUT;
        SetDialogDpiChangeBehavior(window, behavior, behavior);
        let (title, subtitle) = match kind {
            Kind::Speed => ("网络测速", "了解当前连接的真实表现"),
            Kind::Website => ("常用网址", "收藏与管理常用的 SHU 校园服务"),
            Kind::Hotkeys => ("全局快捷键", "为常用操作设置顺手的组合键"),
            Kind::Help => ("使用说明", "SHUNet · 轻量、安静的校园网络助手"),
            Kind::IpInfo => ("IP 信息与出口查询", "本地接口、直连与代理出口 · 查看并复制"),
            Kind::Devices => ("局域网与共享设备", "本机热点、共享下游及其他接口的设备记录"),
            Kind::Status => ("运行状态与记录", "连接概览 · 今日重连统计 · 最近三天记录"),
            Kind::Power => ("定时电源", "关机 · 重启 · 睡眠 · 休眠 · 定时唤醒"),
            Kind::PowerRun => ("电源操作即将执行", "执行前倒计时 · 可随时取消本次操作"),
        };
        SetDlgItemTextW(window, 1600, crate::wide(title).as_ptr());
        SetDlgItemTextW(window, 1601, crate::wide(subtitle).as_ptr());
        let theme = (&*pointer).borrow();
        SendMessageW(window, WM_SETICON, ICON_SMALL as usize, theme.icon as isize);
        SendMessageW(window, WM_SETICON, ICON_BIG as usize, theme.icon as isize);
    }
    unsafe { &*pointer }.borrow_mut().layout(window, true);
    unsafe {
        for (attribute, value) in [(33u32, 2u32), (35, SURFACE), (36, TEXT)] {
            DwmSetWindowAttribute(window, attribute, &value as *const u32 as _, 4);
        }
    }
    crate::dialogs::center_on_cursor(window);
}
pub fn speed_state(window: HWND, progress: f32, tone: Tone, proxy: bool) {
    let pointer = pointer(window);
    if pointer.is_null() {
        return;
    }
    if let Ok(mut theme) = unsafe { &*pointer }.try_borrow_mut() {
        let changed =
            (theme.progress - progress).abs() > 0.001 || theme.tone != tone || theme.proxy != proxy;
        theme.progress = progress;
        theme.tone = tone;
        theme.proxy = proxy;
        if changed {
            unsafe {
                InvalidateRect(window, std::ptr::null(), 0);
                for id in [1401, 1411, 1412] {
                    InvalidateRect(GetDlgItem(window, id), std::ptr::null(), 0);
                }
            }
        }
    }
}
pub fn status_tone(window: HWND, tone: Tone) {
    let pointer=pointer(window);
    if pointer.is_null() { return; }
    if let Ok(mut theme)=unsafe { &*pointer }.try_borrow_mut() {
        if theme.tone!=tone {
            theme.tone=tone;
            unsafe { InvalidateRect(GetDlgItem(window,1811),std::ptr::null(),0); }
        }
    }
}
pub fn message(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<isize> {
    let state = pointer(window);
    if state.is_null() {
        return None;
    }
    if message == WM_NCDESTROY {
        unsafe {
            RemovePropW(window, PROPERTY.as_ptr());
            drop(Box::from_raw(state));
        }
        return None;
    }
    let Ok(mut theme) = unsafe { &*state }.try_borrow_mut() else {
        return None;
    };
    match message {
        WM_ERASEBKGND => Some(1),
        WM_PAINT => {
            theme.paint(window);
            Some(1)
        }
        WM_DRAWITEM if lparam != 0 => {
            draw_item(&theme, unsafe { &*(lparam as *const DRAWITEMSTRUCT) });
            Some(1)
        }
        WM_MEASUREITEM if lparam != 0 => {
            unsafe {
                (*(lparam as *mut MEASUREITEMSTRUCT)).itemHeight = theme.p(32) as u32;
            }
            Some(1)
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
            let id = unsafe { GetDlgCtrlID(lparam as HWND) };
            let (bg, brush) = theme.background(id);
            unsafe {
                SetTextColor(wparam as HDC, theme.text_color(id));
                SetBkColor(wparam as HDC, bg);
            }
            Some(brush as isize)
        }
        WM_DPICHANGED => {
            let progress = theme.progress;
            let tone = theme.tone;
            let proxy = theme.proxy;
            let dpi = (wparam as u32 & 0xffff).max(96);
            let kind = theme.kind;
            let old = std::mem::replace(&mut *theme, Theme::new(kind, dpi));
            theme.progress = progress;
            theme.tone = tone;
            theme.proxy = proxy;
            if lparam != 0 {
                let r = unsafe { &*(lparam as *const RECT) };
                unsafe {
                    SetWindowPos(
                        window,
                        std::ptr::null_mut(),
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
            theme.layout(window, true);
            unsafe {
                SendMessageW(window, WM_SETICON, ICON_SMALL as usize, theme.icon as isize);
                SendMessageW(window, WM_SETICON, ICON_BIG as usize, theme.icon as isize);
            }
            // Child controls have received the new fonts before old resources die.
            drop(old);
            Some(1)
        }
        _ => None,
    }
}
