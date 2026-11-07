//! Familiar text rows and the native status/log submenu. No network I/O.
use crate::{Menu, service::View, menu_theme::{MenuTheme, rgb}};
use std::{cell::RefCell, collections::BTreeSet, rc::Rc};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::Threading::GetCurrentThreadId,
    UI::{Controls::*, HiDpi::*, WindowsAndMessaging::*},
};

thread_local! {
    static DRAW_ROWS: RefCell<BTreeSet<usize>> = const { RefCell::new(BTreeSet::new()) };
    static WIFI_RESOURCES:RefCell<Option<Rc<Resources>>> = const {RefCell::new(None)};
}
#[derive(Clone, Copy)]
enum RowKind { Header(usize), Action, Item, Submenu, Separator }
// Each popup owns its fonts, brush, rows and descendants until native destruction.
// Wi-Fi rebuilding shares this owner instead of leaking owner-draw item pointers.
pub struct Resources {
    pub theme: MenuTheme,
    style: Rc<RefCell<Option<MenuTextStyle>>>,
    pub rows: RefCell<Vec<Box<LiveRow>>>,
    pub children: RefCell<Vec<Menu>>,
    brush: HBRUSH,
}
impl Resources {
    pub fn new(handle:HMENU)->Rc<Self> {
        let theme=crate::menu_theme::current();
        let brush=unsafe { CreateSolidBrush(theme.background()) };
        if theme != MenuTheme::Classic {
            let info=MENUINFO { cbSize:std::mem::size_of::<MENUINFO>() as u32, fMask:MIM_BACKGROUND, hbrBack:brush, ..Default::default() };
            unsafe { SetMenuInfo(handle,&info); }
        }
        Rc::new(Self { theme, style:Rc::new(RefCell::new(None)), rows:RefCell::new(Vec::new()), children:RefCell::new(Vec::new()), brush })
    }
}
impl Drop for Resources { fn drop(&mut self) { unsafe { DeleteObject(self.brush as _); } } }
pub fn set_wifi_resources(value:Option<Rc<Resources>>) { WIFI_RESOURCES.with(|slot| *slot.borrow_mut()=value); }
pub fn wifi_resources()->Option<Rc<Resources>> { WIFI_RESOURCES.with(|slot|slot.borrow().clone()) }
pub struct LiveRow {
    theme: MenuTheme,
    kind: RowKind,
    text: RefCell<String>,
    style: Rc<RefCell<Option<MenuTextStyle>>>,
}
impl LiveRow {
    fn new(text:String, style:Rc<RefCell<Option<MenuTextStyle>>>, theme:MenuTheme, kind:RowKind)->Box<Self> {
        let row=Box::new(Self { text:RefCell::new(text), style, theme, kind });
        DRAW_ROWS.with(|rows| {rows.borrow_mut().insert((&*row as *const Self) as usize);});
        row
    }
}
impl Drop for LiveRow { fn drop(&mut self) {DRAW_ROWS.with(|rows| {rows.borrow_mut().remove(&(self as *const Self as usize));});} }
pub fn static_item(menu:&Menu,id:u32,text:&str,enabled:bool,checked:bool,child:HMENU) {
    let kind=if child.is_null() {RowKind::Item} else {RowKind::Submenu};
    let label=Menu::label(id,text);
    let row=LiveRow::new(label.clone(),menu.2.style.clone(),menu.2.theme,kind);
    let mut label=crate::wide(&label);
    let info=MENUITEMINFOW {
        cbSize:std::mem::size_of::<MENUITEMINFOW>() as u32,
        fMask:MIIM_ID|MIIM_FTYPE|MIIM_STATE|MIIM_DATA|MIIM_STRING|MIIM_SUBMENU,
        fType:MFT_OWNERDRAW, wID:id, hSubMenu:child,
        fState:if enabled {0} else {MFS_DISABLED} | if checked {MFS_CHECKED} else {0},
        dwItemData:(&*row as *const LiveRow) as usize, dwTypeData:label.as_mut_ptr(), ..Default::default()
    };
    unsafe { InsertMenuItemW(menu.0,u32::MAX,1,&info); }
    menu.2.rows.borrow_mut().push(row);
}
pub fn separator(menu:&Menu) {
    let row=LiveRow::new(String::new(),menu.2.style.clone(),menu.2.theme,RowKind::Separator);
    let info=MENUITEMINFOW { cbSize:std::mem::size_of::<MENUITEMINFOW>() as u32, fMask:MIIM_FTYPE|MIIM_STATE|MIIM_DATA,
        fType:MFT_OWNERDRAW|MFT_SEPARATOR, fState:MFS_DISABLED, dwItemData:(&*row as *const LiveRow) as usize, ..Default::default() };
    unsafe { InsertMenuItemW(menu.0,u32::MAX,1,&info); }
    menu.2.rows.borrow_mut().push(row);
}
struct MenuTextStyle {
    font: HFONT,
    bold: HFONT,
    width: u32,
    height: u32,
    dpi: u32,
}
impl Drop for MenuTextStyle {
    fn drop(&mut self) {
        unsafe {
            if !self.font.is_null() {
                DeleteObject(self.font as _);
            }
            if !self.bold.is_null() {
                DeleteObject(self.bold as _);
            }
        }
    }
}
impl MenuTextStyle {
    fn new(window: HWND, theme:MenuTheme) -> Self {
        unsafe {
            let dpi = GetDpiForWindow(window).max(96);
            let p = |value: i32| value * dpi as i32 / 96;
            let mut metrics = NONCLIENTMETRICSW {
                cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
                ..Default::default()
            };
            SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS,
                metrics.cbSize,
                &mut metrics as *mut _ as _,
                0,
                dpi,
            );
            if theme == MenuTheme::Modern {
                metrics.lfMenuFont.lfHeight = -p(12);
                let face=crate::wide("Microsoft YaHei UI");
                metrics.lfMenuFont.lfFaceName.fill(0);
                metrics.lfMenuFont.lfFaceName[..face.len()].copy_from_slice(&face);
            }
            let font = CreateFontIndirectW(&metrics.lfMenuFont);
            let mut bold_info = metrics.lfMenuFont;
            bold_info.lfWeight = 700;
            let bold = CreateFontIndirectW(&bold_info);
            let dc = GetDC(window);
            let previous = SelectObject(dc, font as _);
            let mut text_metrics = TEXTMETRICW::default();
            GetTextMetricsW(dc, &mut text_metrics);
            let measure = |value: &str| {
                let value = crate::wide(value);
                let mut size = SIZE::default();
                GetTextExtentPoint32W(dc, value.as_ptr(), value.len() as i32 - 1, &mut size);
                size.cx
            };
            // Reserve all action captions up front, including shortcut columns.
            let mut width = p(300);
            for (id, text) in [
                (crate::LOGIN, "一键认证"),
                (crate::LOGIN, "正在处理…"),
                (crate::WIFI_SETTINGS, "Wi-Fi 登录设置（802.1X）…"),
                (crate::LOGOUT, "校园网下线（保留 Wi-Fi）"),
                (crate::AUTO_RECONNECT, "自动重连（当前暂停）"),
                (crate::PROXY, "系统代理（已开启）"),
            ] {
                let label = Menu::label(id, text);
                let (name, key) = label.split_once('\t').unwrap_or((&label, ""));
                width = width.max(measure(name) * 11 / 10 + measure(key) + p(36));
            }
            SelectObject(dc, previous);
            ReleaseDC(window, dc);
            Self {
                font,
                bold,
                width: width as u32,
                height: match theme {
                    MenuTheme::Classic => GetSystemMetricsForDpi(SM_CYMENU, dpi).max(text_metrics.tmHeight + p(2)),
                    MenuTheme::Default => GetSystemMetricsForDpi(SM_CYMENU, dpi).max(text_metrics.tmHeight + p(10)),
                    MenuTheme::Modern => p(30).max(text_metrics.tmHeight+p(10)),
                } as u32,
                dpi,
            }
        }
    }
}
pub fn header(menu: &mut Menu, view: &View) {
    let style = menu.2.style.clone();
    for (index,text) in header_text(view).into_iter().enumerate() {
        let row = LiveRow::new(text.clone(),style.clone(),menu.2.theme,RowKind::Header(index));
        let mut label = crate::wide(&Menu::label(0, &text));
        let item = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE | MIIM_STATE | MIIM_DATA | MIIM_STRING,
            fType: MFT_OWNERDRAW,
            fState: MFS_DISABLED,
            dwTypeData: label.as_mut_ptr(),
            dwItemData: (&*row as *const LiveRow) as usize,
            ..Default::default()
        };
        unsafe {
            InsertMenuItemW(menu.0, u32::MAX, 1, &item);
        }
        menu.1.push(row);
    }
}
// Draw only this row, in a memory DC, using the system menu font/theme.
fn draw_row(draw: &DRAWITEMSTRUCT, row: &LiveRow, window: HWND) {
    let Ok(text) = row.text.try_borrow() else {
        return;
    };
    let Ok(mut style) = row.style.try_borrow_mut() else {
        return;
    };
    let style = style.get_or_insert_with(|| MenuTextStyle::new(window, row.theme));
    unsafe {
        let w = draw.rcItem.right - draw.rcItem.left;
        let h = draw.rcItem.bottom - draw.rcItem.top;
        if w <= 0 || h <= 0 {
            return;
        }
        let dc = CreateCompatibleDC(draw.hDC);
        let bitmap = CreateCompatibleBitmap(draw.hDC, w, h);
        if dc.is_null() || bitmap.is_null() {
            if !dc.is_null() {
                DeleteDC(dc);
            }
            if !bitmap.is_null() {
                DeleteObject(bitmap as _);
            }
            return;
        }
        let old_bitmap = SelectObject(dc, bitmap as _);
        let old_font = SelectObject(
            dc,
            if draw.itemState & ODS_DEFAULT != 0 || (row.theme == MenuTheme::Modern && matches!(row.kind, RowKind::Header(0))) {
                style.bold
            } else {
                style.font
            } as _,
        );
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        };
        let selected = draw.itemState & ODS_SELECTED != 0;
        let disabled = draw.itemState & (ODS_DISABLED | ODS_GRAYED) != 0;
        let p = |value: i32| value * style.dpi as i32 / 96;
        let active = selected && !disabled;
        let background = row.theme.background();
        let text_color = match row.theme {
            MenuTheme::Classic => GetSysColor(if disabled {COLOR_GRAYTEXT} else if selected {COLOR_HIGHLIGHTTEXT} else {COLOR_MENUTEXT}),
            MenuTheme::Default => if disabled {rgb(150,150,150)} else {rgb(25,25,25)},
            MenuTheme::Modern => if matches!(row.kind,RowKind::Header(0)) {rgb(30,48,77)}
                else if matches!(row.kind,RowKind::Header(_)) {rgb(104,119,142)}
                else if disabled {rgb(122,135,153)} else if active {rgb(37,86,170)} else {rgb(40,53,73)},
        };
        let brush = CreateSolidBrush(background);
        FillRect(dc,&rect,brush); DeleteObject(brush as _);
        if row.theme == MenuTheme::Classic && selected {
            FillRect(dc,&rect,GetSysColorBrush(COLOR_HIGHLIGHT));
        } else if row.theme == MenuTheme::Modern && matches!(row.kind,RowKind::Header(_)) {
            let brush=CreateSolidBrush(rgb(245,248,253));
            FillRect(dc,&rect,brush);DeleteObject(brush as _);
        } else if active {
            let brush=CreateSolidBrush(if row.theme == MenuTheme::Modern {rgb(234,242,255)} else {rgb(235,235,235)});
            let old_brush=SelectObject(dc,brush as _);let old_pen=SelectObject(dc,GetStockObject(NULL_PEN));
            RoundRect(dc,p(3),1,w-p(3),h-1,p(if row.theme==MenuTheme::Modern {10}else{4}),p(10));
            SelectObject(dc,old_pen);SelectObject(dc,old_brush);DeleteObject(brush as _);
        }
        if matches!(row.kind,RowKind::Separator) {
            let pen=CreatePen(PS_SOLID,1,if row.theme == MenuTheme::Modern {rgb(230,235,243)}else{rgb(215,215,215)});
            let old=SelectObject(dc,pen as _);MoveToEx(dc,p(10),h/2,std::ptr::null_mut());LineTo(dc,w-p(10),h/2);SelectObject(dc,old);DeleteObject(pen as _);
        }
        rect.left += GetSystemMetricsForDpi(SM_CXMENUCHECK, style.dpi) + p(if row.theme==MenuTheme::Modern {12}else{6});
        rect.right -= p(if matches!(row.kind,RowKind::Submenu) {28} else {16});
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc,text_color);
        let flags = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX;
        let (name, key) = text.split_once('\t').unwrap_or((&text, ""));
        if !key.is_empty() {
            let key = crate::wide(key);
            let mut size = SIZE::default();
            GetTextExtentPoint32W(dc, key.as_ptr(), key.len() as i32 - 1, &mut size);
            let mut key_rect = rect;
            key_rect.left = (rect.right - size.cx).max(rect.left);
            DrawTextW(dc, key.as_ptr(), -1, &mut key_rect, flags | DT_RIGHT);
            rect.right = (key_rect.left - 24 * style.dpi as i32 / 96).max(rect.left);
        }
        DrawTextW(dc, crate::wide(&name.replace("&&", "&")).as_ptr(), -1, &mut rect, flags);
        if draw.itemState & ODS_CHECKED != 0 {
            let p = |value: i32| value * style.dpi as i32 / 96;
            let color = if row.theme==MenuTheme::Modern && !disabled {rgb(48,105,218)}else{text_color};
            let pen = CreatePen(PS_SOLID, (style.dpi as i32 + 95) / 96, color);
            let old_pen = SelectObject(dc, pen as _);
            let points = [
                POINT { x: p(4), y: h / 2 },
                POINT {
                    x: p(7),
                    y: h / 2 + p(3),
                },
                POINT {
                    x: p(13),
                    y: h / 2 - p(4),
                },
            ];
            Polyline(dc, points.as_ptr(), points.len() as i32);
            SelectObject(dc, old_pen);
            DeleteObject(pen as _);
        }
        // Windows draws the native submenu arrow after WM_DRAWITEM.
        BitBlt(
            draw.hDC,
            draw.rcItem.left,
            draw.rcItem.top,
            w,
            h,
            dc,
            0,
            0,
            SRCCOPY,
        );
        SelectObject(dc, old_font);
        SelectObject(dc, old_bitmap);
        DeleteObject(bitmap as _);
        DeleteDC(dc);
    }
}
fn header_text(view: &View) -> [String; 4] {
    let connection = if !view.ssid.is_empty() {
        view.ssid.as_str()
    } else {
        match view.state {
            crate::connection_state::ConnectionState::Disconnected => "未连接",
            crate::connection_state::ConnectionState::OtherConnection => {
                "未连接（其他网络接口已连接）"
            }
            _ => "状态未获取",
        }
    };
    [
        format!("Wi-Fi · {connection}"),
        state_text(view),
        view.network_type_text(),
        view.online_duration_text(),
    ]
    .map(|text| {
        text.chars()
            .filter(|character| !character.is_control())
            .collect()
    })
}
fn state_text(view: &View) -> String {
    if view.query.active && view.query.ssid == view.ssid && view.query.ip == view.ip {
        format!("正在查询…（查询前：{}）", view.query.previous_state.label())
    } else {
        view.state.label().into()
    }
}
#[derive(Clone, PartialEq, Eq)]
struct Action {
    id: u32,
    text: String,
    enabled: bool,
    checked: bool,
}
fn actions(view: &View) -> [Action; 4] {
    let enterprise = crate::network::is_enterprise_campus_ssid(&view.ssid);
    let row = |id, label: &str, enabled, checked| Action {
        id,
        text: Menu::label(id, label),
        enabled,
        checked,
    };
    [
        row(
            if enterprise {
                crate::WIFI_SETTINGS
            } else {
                crate::LOGIN
            },
            if enterprise {
                "Wi-Fi 登录设置（802.1X）…"
            } else if view.busy {
                "正在处理…"
            } else {
                "一键认证"
            },
            enterprise || (!view.busy && view.campus),
            false,
        ),
        row(
            crate::LOGOUT,
            "校园网下线（保留 Wi-Fi）",
            !view.busy && view.campus,
            false,
        ),
        row(
            crate::AUTO_RECONNECT,
            if view.auto_reconnect && (!view.status_detection || !view.campus || view.auth_paused) {
                "自动重连（当前暂停）"
            } else {
                "自动重连"
            },
            true,
            view.auto_reconnect,
        ),
        row(
            crate::PROXY,
            if view.proxy {
                "系统代理（已开启）"
            } else {
                "系统代理（已关闭）"
            },
            !view.busy,
            view.proxy,
        ),
    ]
}
pub fn append_actions(menu: &mut Menu, view: &View) {
    let style = menu.1[0].style.clone();
    for action in actions(view) {
        let row = LiveRow::new(action.text.clone(),style.clone(),menu.2.theme,RowKind::Action);
        let mut text = crate::wide(&action.text);
        let item = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_ID | MIIM_FTYPE | MIIM_STATE | MIIM_DATA | MIIM_STRING,
            wID: action.id,
            fType: MFT_OWNERDRAW,
            fState: if action.enabled { 0 } else { MFS_DISABLED }
                | if action.checked { MFS_CHECKED } else { 0 },
            dwTypeData: text.as_mut_ptr(),
            dwItemData: (&*row as *const LiveRow) as usize,
            ..Default::default()
        };
        unsafe {
            InsertMenuItemW(menu.0, u32::MAX, 1, &item);
        }
        menu.1.push(row);
    }
    unsafe {
        SetMenuDefaultItem(
            menu.0,
            if crate::network::is_enterprise_campus_ssid(&view.ssid) {
                crate::WIFI_SETTINGS
            } else {
                crate::LOGIN
            },
            0,
        );
    }
}
struct LiveMenu {
    handle: HMENU,
    rows: [*const LiveRow; 4],
    action_rows: [*const LiveRow; 4],
    actions: [Action; 4],
    desired: [Action; 4],
    children: BTreeSet<usize>,
    visible: bool,
    hovered_popup: bool,
}
thread_local! {static LIVE:RefCell<Option<LiveMenu>>=const{RefCell::new(None)};}
pub const UPDATE_READY: u32 = WM_APP + 31;
pub struct LiveSession<'menu>(std::marker::PhantomData<&'menu Menu>);
impl Drop for LiveSession<'_> {
    fn drop(&mut self) {
        LIVE.with(|slot| {
            *slot.borrow_mut() = None;
        });
    }
}
pub fn watch<'menu>(menu: &'menu Menu, view: &View) -> LiveSession<'menu> {
    let actions = actions(view);
    let rows = std::array::from_fn(|index| &*menu.1[index] as *const LiveRow);
    let action_rows = std::array::from_fn(|index| &*menu.1[index + 4] as *const LiveRow);
    LIVE.with(|slot| {
        *slot.borrow_mut() = Some(LiveMenu {
            handle: menu.0,
            rows,
            action_rows,
            actions: actions.clone(),
            desired: actions,
            children: BTreeSet::new(),
            visible: false,
            hovered_popup: false,
        });
    });
    LiveSession(std::marker::PhantomData)
}
pub fn is_open() -> bool {
    LIVE.with(|slot| slot.try_borrow().is_ok_and(|live| live.is_some()))
}
// All popup lifecycle tracking stays on the owning UI thread. Parent actions
// remain untouched until every child popup has finished its native close.
pub fn message(window: HWND, event: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    if event == WM_MENUSELECT {
        LIVE.with(|slot| {
            let Ok(mut slot) = slot.try_borrow_mut() else {
                return;
            };
            let Some(live) = slot.as_mut() else {
                return;
            };
            let flags = (wparam >> 16) as u32;
            live.hovered_popup = lparam as HMENU == live.handle && flags & MF_POPUP != 0;
            if live.visible && !live.hovered_popup && live.children.is_empty() {
                unsafe {
                    PostMessageW(window, UPDATE_READY, 0, 0);
                }
            }
        });
        return None;
    }
    if event == WM_INITMENUPOPUP || event == WM_UNINITMENUPOPUP {
        LIVE.with(|slot| {
            let Ok(mut slot) = slot.try_borrow_mut() else {
                return;
            };
            let Some(live) = slot.as_mut() else {
                return;
            };
            if wparam as HMENU == live.handle {
                live.visible = event == WM_INITMENUPOPUP;
                if !live.visible {
                    live.children.clear();
                }
            } else if event == WM_INITMENUPOPUP {
                live.children.insert(wparam);
            } else {
                live.children.remove(&wparam);
                if live.visible && live.children.is_empty() {
                    unsafe {
                        PostMessageW(window, UPDATE_READY, 0, 0);
                    }
                }
            }
        });
        return None;
    }
    if event == UPDATE_READY {
        flush_actions();
        return Some(0);
    }
    if lparam == 0 || !matches!(event, WM_MEASUREITEM | WM_DRAWITEM) {
        return None;
    }
    unsafe {
        let data = if event == WM_MEASUREITEM {
            let item = &*(lparam as *const MEASUREITEMSTRUCT);
            if item.CtlType != ODT_MENU {
                return None;
            }
            item.itemData
        } else {
            let item = &*(lparam as *const DRAWITEMSTRUCT);
            if item.CtlType != ODT_MENU {
                return None;
            }
            item.itemData
        };
        let valid = DRAW_ROWS.with(|rows|rows.try_borrow().is_ok_and(|rows|rows.contains(&data)));
        if !valid {
            return None;
        }
        let row = &*(data as *const LiveRow);
        if event == WM_MEASUREITEM {
            let Ok(mut style) = row.style.try_borrow_mut() else {
                return Some(1);
            };
            let style = style.get_or_insert_with(|| MenuTextStyle::new(window, row.theme));
            let item = &mut *(lparam as *mut MEASUREITEMSTRUCT);
            item.itemWidth = if matches!(row.kind,RowKind::Header(_)|RowKind::Action) {style.width} else {1};
            if matches!(row.kind,RowKind::Item|RowKind::Submenu) {
                let dc=GetDC(window); let old=SelectObject(dc,style.bold as _);
                let text=row.text.borrow(); let (name,key)=text.split_once('\t').unwrap_or((&text,""));
                let measure=|value:&str| {let value=crate::wide(value);let mut size=SIZE::default();GetTextExtentPoint32W(dc,value.as_ptr(),value.len() as i32-1,&mut size);size.cx};
                item.itemWidth=item.itemWidth.max((measure(name)+measure(key)+80*style.dpi as i32/96).min(760*style.dpi as i32/96) as u32);
                SelectObject(dc,old);ReleaseDC(window,dc);
            }
            item.itemHeight = if matches!(row.kind,RowKind::Separator) { 9*style.dpi/96 } else {style.height};
        } else {
            draw_row(&*(lparam as *const DRAWITEMSTRUCT), row, window);
        }
    }
    Some(1)
}
pub fn refresh(view: &View) {
    let repaint = LIVE.with(|slot| {
        let Ok(mut slot) = slot.try_borrow_mut() else {
            return None;
        };
        let live = slot.as_mut()?;
        let mut changed = Vec::new();
        for (position, text) in header_text(view).into_iter().enumerate() {
            let row = unsafe { &*live.rows[position] };
            if let Ok(mut current) = row.text.try_borrow_mut() {
                if *current != text {
                    *current = text;
                    changed.push(position as u32);
                }
            }
        }
        live.desired = actions(view);
        Some((live.handle, changed))
    });
    if let Some((menu, mut positions)) = repaint {
        apply_actions(menu, &mut positions);
        invalidate_rows(menu, positions);
    }
}
fn flush_actions() {
    let menu = LIVE.with(|slot| {
        slot.try_borrow()
            .ok()
            .and_then(|slot| slot.as_ref().map(|live| live.handle))
    });
    if let Some(menu) = menu {
        let mut positions = Vec::new();
        apply_actions(menu, &mut positions);
        invalidate_rows(menu, positions);
    }
}
fn apply_actions(menu: HMENU, changed: &mut Vec<u32>) {
    for index in 0..4 {
        let pending = LIVE.with(|slot| {
            let slot = slot.try_borrow().ok()?;
            let live = slot.as_ref()?;
            if live.handle != menu
                || !live.visible
                || live.hovered_popup
                || !live.children.is_empty()
            {
                return None;
            }
            (live.desired[index] != live.actions[index]).then(|| {
                (
                    live.desired[index].clone(),
                    live.actions[index].clone(),
                    live.action_rows[index],
                )
            })
        });
        let Some((desired, previous, pointer)) = pending else {
            continue;
        };
        let row = unsafe { &*pointer };
        row.text.replace(desired.text.clone());
        // Native enable/check updates may synchronously request painting. Never
        // hold LIVE or the row text borrowed while calling back into Windows.
        if update_action(menu, index as u32 + 5, &desired, &previous) {
            LIVE.with(|slot| {
                if let Ok(mut slot) = slot.try_borrow_mut() {
                    if let Some(live) = slot.as_mut().filter(|live| live.handle == menu) {
                        live.actions[index] = desired;
                    }
                }
            });
            changed.push(index as u32 + 5);
        } else {
            row.text.replace(previous.text);
        }
    }
}
fn update_action(menu: HMENU, position: u32, desired: &Action, previous: &Action) -> bool {
    unsafe {
        if desired.enabled != previous.enabled {
            let result = EnableMenuItem(
                menu,
                position,
                MF_BYPOSITION
                    | if desired.enabled {
                        MF_ENABLED
                    } else {
                        MF_GRAYED
                    },
            );
            if result == -1 {
                #[cfg(test)]
                println!(
                    "EnableMenuItem failed position={position}, error={}",
                    GetLastError()
                );
                return false;
            }
        }
        if desired.checked != previous.checked {
            if CheckMenuItem(
                menu,
                position,
                MF_BYPOSITION
                    | if desired.checked {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    },
            ) == u32::MAX
            {
                #[cfg(test)]
                println!(
                    "CheckMenuItem failed position={position}, error={}",
                    GetLastError()
                );
                return false;
            }
        }
        if desired.id != previous.id {
            let info = MENUITEMINFOW {
                cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_ID,
                wID: desired.id,
                ..Default::default()
            };
            if SetMenuItemInfoW(menu, position, 1, &info) == 0 {
                return false;
            }
        }
        true
    }
}
struct Repaint {
    menu: HMENU,
    positions: Vec<u32>,
}
fn invalidate_rows(menu: HMENU, positions: Vec<u32>) {
    if positions.is_empty() {
        return;
    }
    let data = Repaint { menu, positions };
    unsafe {
        EnumThreadWindows(
            GetCurrentThreadId(),
            Some(invalidate_popup),
            &data as *const Repaint as isize,
        );
    }
}
unsafe extern "system" fn invalidate_popup(window: HWND, parameter: LPARAM) -> i32 {
    unsafe {
        if IsWindowVisible(window) == 0 {
            return 1;
        }
        let mut class = [0u16; 16];
        let length = GetClassNameW(window, class.as_mut_ptr(), 16);
        if String::from_utf16_lossy(&class[..length.max(0) as usize]) != "#32768" {
            return 1;
        }
        let data = &*(parameter as *const Repaint);
        let mut info = MENUBARINFO {
            cbSize: std::mem::size_of::<MENUBARINFO>() as u32,
            ..Default::default()
        };
        if GetMenuBarInfo(window, OBJID_CLIENT, 0, &mut info) != 0 && info.hMenu == data.menu {
            for position in &data.positions {
                let mut rect = RECT::default();
                if GetMenuItemRect(std::ptr::null_mut(), data.menu, *position, &mut rect) != 0 {
                    MapWindowPoints(
                        std::ptr::null_mut(),
                        window,
                        &mut rect as *mut RECT as *mut POINT,
                        2,
                    );
                    // No whole-menu repaint, background erase, synchronous paint
                    // or layout mutation for the live clock/status rows.
                    InvalidateRect(window, &rect, 0);
                }
            }
        }
    }
    1
}

pub fn status_menu(view: &View) -> Menu {
    let info = Menu::new();
    info.item(0, &view.network_type_text(), false, false);
    info.item(0, &view.online_duration_text(), false, false);
    info.item(
        0,
        &format!("所选运营商：{}（下次认证生效）", view.operator.label()),
        false,
        false,
    );
    info.item(
        0,
        &format!("SHUNet v{}", env!("CARGO_PKG_VERSION")),
        false,
        false,
    );
    info.item(0, &format!("认证状态：{}", state_text(view)), false, false);
    info.item(0, &format!("最近检测：{}", view.checked_at), false, false);
    let now = crate::runtime_history::now_ms();
    let stats = view.history.stats(now);
    info.item(
        0,
        &format!(
            "今日自动重连：{} 次（已确认恢复 {} 次）",
            stats.attempts, stats.recovered
        ),
        false,
        false,
    );
    info.item(
        0,
        &format!("检测模式：{}", view.detection_level.label()),
        false,
        false,
    );
    if let Some(next) = view.next_detection {
        info.item(
            0,
            &format!("本轮间隔：{} 秒", view.detection_interval),
            false,
            false,
        );
        let seconds = next
            .saturating_duration_since(std::time::Instant::now())
            .as_secs_f64()
            .ceil() as u64;
        let text = if view.busy {
            "下次查询：等待当前操作结束".into()
        } else if seconds == 0 {
            "下次查询：已到计划时间".into()
        } else {
            format!("下次查询：约 {seconds} 秒后（打开菜单时）")
        };
        info.item(0, &text, false, false);
    }
    info.item(
        0,
        &format!("调度原因：{}", view.detection_reason),
        false,
        false,
    );
    if !view.confirmed_at.is_empty() {
        info.item(
            0,
            &format!("上次确认在线：{}", view.confirmed_at),
            false,
            false,
        );
    }
    info.item(
        0,
        &view.state_detail.chars().take(80).collect::<String>(),
        false,
        false,
    );
    if let Some(warning) = &view.history_warning {
        info.item(0, warning, false, false);
    }
    info.line();
    info.item(crate::STATUS_WINDOW, "查看详细信息…", true, false);
    info.item(crate::ICON_LEGEND, "图标状态说明…", true, false);
    info.line();
    info.item(
        crate::COPY_IP,
        &format!("复制本机 IP：{}", view.ip),
        view.ip != "—",
        false,
    );
    info.item(
        crate::COPY_LOGS,
        "复制运行记录",
        !view.history.is_empty() || !view.logs.is_empty(),
        false,
    );
    info.line();
    if !view.history.is_empty() {
        let today = crate::runtime_history::day(now);
        for entry in view.history.iter().rev().take(6) {
            let format = if crate::runtime_history::day(entry.timestamp) == today {
                "%H:%M:%S"
            } else {
                "%m-%d %H:%M:%S"
            };
            let text = format!(
                "[{}] {}",
                crate::runtime_history::shanghai_time(entry.timestamp, format),
                entry.text
            );
            info.item(0, &text.chars().take(70).collect::<String>(), false, false);
        }
    } else if !view.logs.is_empty() {
        for text in view.logs.iter().rev().take(6) {
            info.item(0, &text.chars().take(70).collect::<String>(), false, false);
        }
    } else {
        info.item(0, "暂无运行记录", false, false);
    }
    info
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use windows_sys::Win32::System::StationsAndDesktops::*;
    pub(crate) struct TestDesktop {
        previous: HDESK,
        created: HDESK,
    }
    impl TestDesktop {
        pub(crate) fn new() -> Self {
            unsafe {
                let previous = GetThreadDesktop(GetCurrentThreadId());
                let name = crate::wide(&format!(
                    "SHUNet.MenuTest.{}.{}",
                    std::process::id(),
                    GetCurrentThreadId()
                ));
                let created = CreateDesktopW(
                    name.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                    DESKTOP_CREATEWINDOW
                        | DESKTOP_CREATEMENU
                        | DESKTOP_READOBJECTS
                        | DESKTOP_WRITEOBJECTS
                        | DESKTOP_ENUMERATE,
                    std::ptr::null(),
                );
                assert!(
                    !created.is_null(),
                    "create own hidden test desktop: {}",
                    GetLastError()
                );
                assert_ne!(
                    SetThreadDesktop(created),
                    0,
                    "assign test thread desktop: {}",
                    GetLastError()
                );
                Self { previous, created }
            }
        }
    }
    impl Drop for TestDesktop {
        fn drop(&mut self) {
            unsafe {
                if SetThreadDesktop(self.previous) != 0 {
                    CloseDesktop(self.created);
                }
            }
        }
    }
    static CAPTURED: AtomicU32 = AtomicU32::new(0);
    static UPDATED: AtomicU32 = AtomicU32::new(0);
    fn test_position(_: HWND) -> POINT {
        POINT { x: 80, y: 80 }
    }

    thread_local! { static TEST_VIEW:RefCell<Option<View>>=const{RefCell::new(None)}; }
    unsafe extern "system" fn complete(window: HWND, _: u32, id: usize, _: u32) {
        unsafe {
            KillTimer(window, id);
        }
        TEST_VIEW.with(|slot| {
            if let Some(view) = slot.borrow_mut().as_mut() {
                view.busy = false;
                view.state = crate::connection_state::ConnectionState::Online;
                view.auth_paused = false;
                view.proxy = true;
            }
        });
        unsafe {
            PostMessageW(window, crate::SERVICE_EVENT, 0, 0);
        }
    }
    fn menu_text(menu: HMENU, position: u32) -> String {
        if position < 4 || (5..=8).contains(&position) {
            let mut info = MENUITEMINFOW {
                cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_DATA,
                ..Default::default()
            };
            if unsafe { GetMenuItemInfoW(menu, position, 1, &mut info) } != 0
                && info.dwItemData != 0
            {
                return unsafe { &*(info.dwItemData as *const LiveRow) }
                    .text
                    .borrow()
                    .clone();
            }
        }
        let mut text = vec![0u16; 1024];
        let n = unsafe {
            GetMenuStringW(
                menu,
                position,
                text.as_mut_ptr(),
                text.len() as i32,
                MF_BYPOSITION,
            )
        };
        String::from_utf16_lossy(&text[..n.max(0) as usize])
    }
    unsafe extern "system" fn owner(
        window: HWND,
        event: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        STRESS.with(|slot| {
            if let Some(state) = slot.borrow_mut().as_mut() {
                if wparam as HMENU == state.child {
                    if event == WM_INITMENUPOPUP {
                        state.seen_child = true;
                        state.hierarchical = lparam & 0xffff != 0;
                    }
                    if event == WM_UNINITMENUPOPUP && !state.finishing {
                        state.closed_early = true;
                    }
                }
            }
        });
        if let Some(result) = message(window, event, wparam, lparam) {
            return result;
        }
        if event == crate::SERVICE_EVENT || (event == WM_TIMER && wparam == 1) {
            TEST_VIEW.with(|slot| {
                if let Some(view) = slot.borrow().as_ref() {
                    refresh(view);
                }
            });
            UPDATED.fetch_add(1, Ordering::Relaxed);
            return 0;
        }
        unsafe { DefWindowProcW(window, event, wparam, lparam) }
    }
    unsafe extern "system" fn capture_popup(window: HWND, _: LPARAM) -> i32 {
        let mut class = [0u16; 32];
        let n = unsafe { GetClassNameW(window, class.as_mut_ptr(), 32) };
        if String::from_utf16_lossy(&class[..n.max(0) as usize]) == "#32768"
            && unsafe { IsWindowVisible(window) } != 0
        {
            crate::network_tools_dialog::tests::capture(
                window,
                if CAPTURED.load(Ordering::Relaxed) == 0 {
                    "live-main-menu"
                } else {
                    "live-status-menu"
                },
            );
            CAPTURED.fetch_add(1, Ordering::Relaxed);
        }
        1
    }
    unsafe extern "system" fn finish(window: HWND, _: u32, id: usize, _: u32) {
        unsafe {
            KillTimer(window, id);
            EnumThreadWindows(
                windows_sys::Win32::System::Threading::GetCurrentThreadId(),
                Some(capture_popup),
                0,
            );
            EndMenu();
        }
    }
    #[test]
    fn theme_selector_and_wifi_rebuild_preserve_ownership_and_commands() {
        for theme in MenuTheme::ALL {
            let _theme=crate::menu_theme::Session::new(theme);
            let root=Menu::new();
            let choices=crate::theme_menu(theme);
            for (index,choice) in MenuTheme::ALL.into_iter().enumerate() {
                unsafe {
                    assert_eq!(GetMenuItemID(choices.0,index as i32),crate::THEME_DEFAULT+index as u32);
                    assert_eq!(GetMenuState(choices.0,index as u32,MF_BYPOSITION)&MF_CHECKED != 0,choice==theme);
                }
            }
            root.submenu("菜单主题",choices);
            let wifi=Menu::new();let handle=wifi.0;let resources=Rc::downgrade(&wifi.2);
            let session=crate::WifiMenuSession::new(&wifi);
            root.submenu("Wi-Fi",wifi);
            let view=View {ssid:"Shu(ForAll)".into(),scanning:false,scan_detail:"synthetic scan".into(),..View::default()};
            let mut expected_rows=None;
            for _ in 0..20 {
                crate::fill_wifi_menu(handle,&view);
                assert_eq!(resources.upgrade().unwrap().theme,theme);
                let rows=DRAW_ROWS.with(|rows|rows.borrow().len());
                if let Some(expected)=expected_rows {assert_eq!(rows,expected,"rebuilding must reclaim old item data");} else {expected_rows=Some(rows);}
                unsafe {assert_eq!(GetMenuItemID(handle,0),crate::SCAN);assert_eq!(IsMenu(handle),1);}
            }
            drop(session);drop(root);
            assert!(resources.upgrade().is_none());
            unsafe {assert_eq!(IsMenu(handle),0);}
            DRAW_ROWS.with(|rows|assert!(rows.borrow().is_empty()));
        }
    }

    #[test]
    #[ignore = "Opens and captures only an own native popup with synthetic state, then closes automatically; no network calls"]
    fn native_menu_summary_smoke() {
        let theme=std::env::var("SHUNET_TEST_MENU_THEME").ok().and_then(|value|serde_json::from_value(serde_json::Value::String(value)).ok()).unwrap_or_default();
        let _theme=crate::menu_theme::Session::new(theme);
        let _desktop = TestDesktop::new();
        unsafe {
            windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
                windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            );
            let module =
                windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
            let name = crate::wide("SHUNet.MenuPreview.Test");
            assert_ne!(
                RegisterClassW(&WNDCLASSW {
                    lpfnWndProc: Some(owner),
                    hInstance: module,
                    lpszClassName: name.as_ptr(),
                    ..Default::default()
                }),
                0
            );
            let window = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                name.as_ptr(),
                name.as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                module,
                std::ptr::null(),
            );
            assert!(!window.is_null());
            let mut view = crate::service::View {
                ssid: "Shu(ForAll)".into(),
                campus: true,
                state: crate::connection_state::ConnectionState::Online,
                ..Default::default()
            };
            view.online_session.confirm(None);
            view.history = std::sync::Arc::new(vec![crate::runtime_history::Entry {
                id: 1,
                timestamp: crate::runtime_history::now_ms(),
                event: crate::runtime_history::Event::Message,
                attempt: None,
                text: "手动查询完成：门户已确认认证在线。".into(),
            }].into());
            view.checked_at = "18:20:00".into();
            view.confirmed_at = "18:20:00".into();
            view.next_detection =
                Some(std::time::Instant::now() + std::time::Duration::from_secs(10));
            view.detection_interval = 10;
            view.detection_reason = "按所选固定档位查询".into();
            view.state_detail = "已通过当前物理 Wi-Fi 查询并确认本机会话在线；无需重复登录".into();
            view.busy = true;
            view.state = crate::connection_state::ConnectionState::Authenticating;
            view.auto_reconnect = true;
            view.auth_paused = true;
            let mut menu = crate::Menu::new();
            header(&mut menu, &view);
            menu.line();
            append_actions(&mut menu, &view);
            menu.line();
            for label in ["Wi-Fi 网络", "SHU 大常用网址", "校园网工具"] {
                let child = crate::Menu::new();
                child.item(80, "预览项目", true, false);
                menu.submenu(label, child);
            }
            let status = status_menu(&view);
            let status_handle = status.0;
            menu.submenu("运行状态与记录", status);
            let settings = crate::Menu::new();
            settings.item(81, "账号与密码", true, false);
            settings.submenu("菜单主题",crate::theme_menu(theme));
            menu.submenu("设置", settings);
            menu.line();
            menu.item(99, "退出", true, false);
            let before = menu_text(menu.0, 3);
            assert_eq!(menu_text(menu.0, 5), "正在处理…");
            assert_ne!(GetMenuState(menu.0, 5, MF_BYPOSITION) & MF_GRAYED, 0);
            let count = GetMenuItemCount(menu.0);
            let live = watch(&menu, &view);
            TEST_VIEW.with(|slot| {
                *slot.borrow_mut() = Some(view.clone());
            });
            SetTimer(window, 1, 1000, None);
            SetTimer(window, 8, 450, Some(complete));
            SetForegroundWindow(window);
            assert_ne!(SetTimer(window, 9, 2600, Some(finish)), 0);
            let position = test_position(window);
            TrackPopupMenu(
                menu.0,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                position.x,
                position.y,
                0,
                window,
                std::ptr::null(),
            );
            KillTimer(window, 1);
            KillTimer(window, 8);
            assert!(
                UPDATED.load(Ordering::Relaxed) >= 3,
                "owner timer and service events must execute while popup stays open"
            );
            assert_ne!(
                menu_text(menu.0, 3),
                before,
                "online duration must advance without reopening menu"
            );
            assert_eq!(
                menu_text(menu.0, 1),
                crate::connection_state::ConnectionState::Online.label()
            );
            assert_eq!(menu_text(menu.0, 5), "一键认证");
            assert_eq!(
                GetMenuState(menu.0, 5, MF_BYPOSITION) & MF_GRAYED,
                0,
                "completed login must become clickable"
            );
            assert_eq!(menu_text(menu.0, 7), "自动重连");
            assert_eq!(menu_text(menu.0, 8), "系统代理（已开启）");
            assert_ne!(GetMenuState(menu.0, 8, MF_BYPOSITION) & MF_CHECKED, 0);
            assert_eq!(GetMenuItemID(menu.0, 5), crate::LOGIN);
            assert_eq!(
                GetMenuItemCount(menu.0),
                count,
                "refresh must preserve menu structure"
            );
            drop(live);
            assert!(!is_open());
            TEST_VIEW.with(|slot| {
                *slot.borrow_mut() = None;
            });
            assert_eq!(
                CAPTURED.load(Ordering::Relaxed),
                1,
                "main menu must display successfully"
            );
            assert_ne!(SetTimer(window, 9, 500, Some(finish)), 0);
            TrackPopupMenu(
                status_handle,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                80,
                80,
                0,
                window,
                std::ptr::null(),
            );
            KillTimer(window, 9);
            DestroyWindow(window);
            UnregisterClassW(name.as_ptr(), module);
            assert!(
                CAPTURED.load(Ordering::Relaxed) == 2,
                "own menu screenshot must be captured"
            );
            drop(menu);
            DRAW_ROWS.with(|rows|assert!(rows.borrow().is_empty(),"all root and descendant drawing data must be reclaimed"));
        }
    }
    struct Stability {
        root: HMENU,
        child: HMENU,
        ticks: u32,
        seen_child: bool,
        hierarchical: bool,
        child_samples: u32,
        finishing: bool,
        closed_early: bool,
        geometry_changed: bool,
        root_rect: Option<[i32; 4]>,
        child_rect: Option<[i32; 4]>,
    }
    thread_local! {static STRESS:RefCell<Option<Stability>>=const{RefCell::new(None)};}
    fn popup(menu: HMENU) -> HWND {
        struct Find {
            menu: HMENU,
            window: HWND,
        }
        unsafe extern "system" fn visit(window: HWND, data: LPARAM) -> i32 {
            unsafe {
                if IsWindowVisible(window) == 0 {
                    return 1;
                }
                let state = &mut *(data as *mut Find);
                let mut info = MENUBARINFO {
                    cbSize: std::mem::size_of::<MENUBARINFO>() as u32,
                    ..Default::default()
                };
                if GetMenuBarInfo(window, OBJID_CLIENT, 0, &mut info) != 0
                    && info.hMenu == state.menu
                {
                    state.window = window;
                    return 0;
                }
                1
            }
        }
        let mut state = Find {
            menu,
            window: std::ptr::null_mut(),
        };
        unsafe {
            EnumThreadWindows(
                GetCurrentThreadId(),
                Some(visit),
                &mut state as *mut Find as isize,
            );
        }
        state.window
    }
    fn rectangle(window: HWND) -> [i32; 4] {
        let mut rect = RECT::default();
        unsafe {
            GetWindowRect(window, &mut rect);
        }
        [rect.left, rect.top, rect.right, rect.bottom]
    }
    unsafe extern "system" fn stress_timer(window: HWND, _: u32, id: usize, _: u32) {
        let result = std::panic::catch_unwind(|| {
            let (root, child, ticks) = STRESS.with(|slot| {
                let mut slot = slot.borrow_mut();
                let state = slot.as_mut().unwrap();
                state.ticks += 1;
                (state.root, state.child, state.ticks)
            });
            let root_window = popup(root);
            let child_window = popup(child);
            STRESS.with(|slot| {
                let mut slot = slot.borrow_mut();
                let state = slot.as_mut().unwrap();
                if root_window.is_null() {
                    state.closed_early = true;
                } else {
                    let rect = rectangle(root_window);
                    if state.root_rect.is_some_and(|old| old != rect) {
                        state.geometry_changed = true;
                    }
                    state.root_rect.get_or_insert(rect);
                }
                if !child_window.is_null() {
                    state.child_samples += 1;
                    let rect = rectangle(child_window);
                    if state.child_rect.is_some_and(|old| old != rect) {
                        state.geometry_changed = true;
                    }
                    state.child_rect.get_or_insert(rect);
                } else if state.child_samples > 0 {
                    state.closed_early = true;
                }
            });
            TEST_VIEW.with(|slot| {
                if let Some(view) = slot.borrow_mut().as_mut() {
                    view.query.active = ticks % 2 == 0;
                    view.state = if ticks % 3 == 0 {
                        crate::connection_state::ConnectionState::OnlineRechecking
                    } else { crate::connection_state::ConnectionState::Online };
                    view.query.ssid = view.ssid.clone();
                    view.query.ip = view.ip.clone();
                    view.query.previous_state =
                        crate::connection_state::ConnectionState::GatewayUnavailable;
                    view.busy = ticks % 3 == 0;
                    view.proxy = ticks % 2 == 0;
                    view.auth_paused = ticks % 2 == 0;
                }
            });
            unsafe {
                PostMessageW(window, crate::SERVICE_EVENT, 0, 0);
            }
            if ticks == 5 {
                use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
                for key in [VK_END, VK_UP, VK_UP, VK_RIGHT] {
                    unsafe {
                        PostMessageW(root_window, WM_KEYDOWN, key as usize, 0);
                    }
                }
            }
            let seen_child = STRESS.with(|slot| slot.borrow().as_ref().unwrap().seen_child);
            if ticks == 9 && !seen_child && !root_window.is_null() {
                let rect = rectangle(root_window);
                unsafe {
                    TrackPopupMenuEx(
                        child,
                        TPM_RECURSE | TPM_RETURNCMD | TPM_RIGHTBUTTON,
                        rect[2] - 2,
                        rect[1],
                        window,
                        std::ptr::null(),
                    );
                }
                STRESS.with(|slot| {
                    let mut slot = slot.borrow_mut();
                    let state = slot.as_mut().unwrap();
                    if !state.finishing {
                        state.closed_early = true;
                        state.finishing = true;
                    }
                });
                unsafe {
                    KillTimer(window, id);
                    EndMenu();
                }
                return;
            }
            if ticks >= 60 {
                STRESS.with(|slot| {
                    slot.borrow_mut().as_mut().unwrap().finishing = true;
                });
                if !root_window.is_null() {
                    crate::network_tools_dialog::tests::capture(root_window, "stable-menu-root");
                }
                if !child_window.is_null() {
                    crate::network_tools_dialog::tests::capture(child_window, "stable-menu-child");
                }
                unsafe {
                    KillTimer(window, id);
                    EndMenu();
                }
            }
        });
        if result.is_err() {
            unsafe {
                KillTimer(window, id);
                EndMenu();
            }
        }
    }
    #[test]
    #[ignore = "Keeps an own native submenu open across 60 live updates; synthetic data only, no network or external application input"]
    fn native_menu_geometry_and_submenu_stress() {
        let theme=std::env::var("SHUNET_TEST_MENU_THEME").ok().and_then(|value|serde_json::from_value(serde_json::Value::String(value)).ok()).unwrap_or_default();
        let _theme=crate::menu_theme::Session::new(theme);
        let _desktop = TestDesktop::new();
        unsafe {
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let module =
                windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
            let name = crate::wide("SHUNet.StableMenu.Test");
            assert_ne!(
                RegisterClassW(&WNDCLASSW {
                    lpfnWndProc: Some(owner),
                    hInstance: module,
                    lpszClassName: name.as_ptr(),
                    ..Default::default()
                }),
                0
            );
            let window = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                name.as_ptr(),
                name.as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                module,
                std::ptr::null(),
            );
            assert!(!window.is_null());
            let mut view = View {
                ssid: "Shu(ForAll)".into(),
                ip: "10.98.1.2".into(),
                campus: true,
                state: crate::connection_state::ConnectionState::Online,
                auto_reconnect: true,
                ..View::default()
            };
            view.online_session.confirm(None);
            let mut root = Menu::new();
            header(&mut root, &view);
            root.line();
            append_actions(&mut root, &view);
            root.line();
            for label in ["Wi-Fi 网络", "SHU 大常用网址", "校园网工具"] {
                let child = Menu::new();
                child.item(80, "预览项目", true, false);
                root.submenu(label, child);
            }
            let child = status_menu(&view);
            let child_handle = child.0;
            root.submenu("运行状态与记录", child);
            let settings = Menu::new();
            settings.item(81, "账号与密码", true, false);
            settings.submenu("菜单主题",crate::theme_menu(theme));
            root.submenu("设置", settings);
            root.line();
            root.item(99, "退出", true, false);
            let live = watch(&root, &view);
            TEST_VIEW.with(|slot| {
                *slot.borrow_mut() = Some(view);
            });
            STRESS.with(|slot| {
                *slot.borrow_mut() = Some(Stability {
                    root: root.0,
                    child: child_handle,
                    ticks: 0,
                    seen_child: false,
                    hierarchical: false,
                    child_samples: 0,
                    finishing: false,
                    closed_early: false,
                    geometry_changed: false,
                    root_rect: None,
                    child_rect: None,
                });
            });
            SetWindowPos(window, std::ptr::null_mut(), 40, 40, 200, 80, SWP_NOZORDER);
            ShowWindow(window, SW_SHOW);
            ShowWindow(window, SW_RESTORE);
            SetForegroundWindow(window);
            SetTimer(window, 6, 150, Some(stress_timer));
            let position = test_position(window);
            let selected = TrackPopupMenu(
                root.0,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                position.x,
                position.y,
                0,
                window,
                std::ptr::null(),
            );
            println!("stress menu returned command={selected}");
            KillTimer(window, 6);
            let state = STRESS.with(|slot| slot.borrow_mut().take().unwrap());
            drop(live);
            TEST_VIEW.with(|slot| {
                *slot.borrow_mut() = None;
            });
            DestroyWindow(window);
            UnregisterClassW(name.as_ptr(), module);
            println!(
                "updates={}, child_samples={}, child_opened={}, hierarchical={}, closed_early={}, geometry_changed={}",
                state.ticks,
                state.child_samples,
                state.seen_child,
                state.hierarchical,
                state.closed_early,
                state.geometry_changed
            );
            assert!(
                state.finishing && state.ticks >= 60,
                "menu must remain open throughout all updates"
            );
            assert!(
                state.seen_child && state.child_samples >= 40,
                "actual native status submenu must stay open"
            );
            assert!(
                !state.closed_early,
                "refresh must not close the child or parent popup"
            );
            assert!(
                !state.geometry_changed,
                "menu rectangles must stay fixed during short/long captions and action changes"
            );
        }
    }
}
