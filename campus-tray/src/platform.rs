use std::io;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Networking::WinInet::{
    INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED, InternetSetOptionW,
};
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};

const INTERNET_SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

pub fn copy_text(text: &str) -> Result<(), String> {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err("剪贴板暂不可用，请重试。".to_owned());
        }
        let result = (|| {
            let memory = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
            if memory.is_null() {
                return Err("无法分配剪贴板内存。".to_owned());
            }
            let pointer = GlobalLock(memory).cast::<u16>();
            if pointer.is_null() {
                windows_sys::Win32::Foundation::GlobalFree(memory);
                return Err("无法锁定剪贴板内存。".to_owned());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), pointer, wide.len());
            GlobalUnlock(memory);
            if EmptyClipboard() == 0 || SetClipboardData(13, memory).is_null() {
                windows_sys::Win32::Foundation::GlobalFree(memory);
                return Err("无法写入剪贴板。".to_owned());
            }
            Ok(())
        })();
        CloseClipboard();
        result
    }
}

pub fn open(target: &str) {
    let _ = try_open(target);
}

pub fn try_open(target: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let target: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
    let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        ) as isize
    };
    if result <= 32 {
        Err("无法打开网页或系统设置，请检查默认浏览器。".to_owned())
    } else {
        Ok(())
    }
}

pub fn prompt_credentials(
    owner: windows_sys::Win32::Foundation::HWND,
    username: &str,
    current_password: &str,
    remember_password: bool,
) -> Result<Option<(String, String, bool)>, String> {
    use windows_sys::Win32::Security::Credentials::*;
    let mut user = [0u16; 514];
    let mut password = [0u16; 1025];
    fill_credential_buffer(&mut user, username)?;
    fill_credential_buffer(&mut password, current_password)?;
    let title: Vec<u16> = "SHUNet · 校园网账号"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let message: Vec<u16> = "当前密码会以掩码显示，可直接修改。勾选记住后使用 Windows DPAPI 加密保存在本机；不会验证或修改 Windows 登录密码。".encode_utf16().chain(Some(0)).collect();
    let target: Vec<u16> = "SHUNet".encode_utf16().chain(Some(0)).collect();
    let info = CREDUI_INFOW {
        cbSize: std::mem::size_of::<CREDUI_INFOW>() as u32,
        hwndParent: owner,
        pszMessageText: message.as_ptr(),
        pszCaptionText: title.as_ptr(),
        hbmBanner: std::ptr::null_mut(),
    };
    let mut remember = i32::from(remember_password);
    let code = unsafe {
        CredUIPromptForCredentialsW(
            &info,
            target.as_ptr(),
            std::ptr::null(),
            0,
            user.as_mut_ptr(),
            user.len() as u32,
            password.as_mut_ptr(),
            password.len() as u32,
            &mut remember,
            CREDUI_FLAGS_GENERIC_CREDENTIALS
                | CREDUI_FLAGS_ALWAYS_SHOW_UI
                | CREDUI_FLAGS_DO_NOT_PERSIST
                | CREDUI_FLAGS_SHOW_SAVE_CHECK_BOX,
        )
    };
    let result = if code == 0 {
        let username = String::from_utf16_lossy(
            &user[..user.iter().position(|ch| *ch == 0).unwrap_or(user.len())],
        );
        let secret = String::from_utf16_lossy(
            &password[..password
                .iter()
                .position(|ch| *ch == 0)
                .unwrap_or(password.len())],
        );
        if username.trim().is_empty() || secret.is_empty() {
            Err("账号和密码不能为空。".to_owned())
        } else {
            Ok(Some((username, secret, remember != 0)))
        }
    } else if code == 1223 {
        Ok(None)
    } else {
        Err(format!("无法打开账号设置（Windows 错误 {code}）。"))
    };
    for ch in &mut password {
        unsafe {
            std::ptr::write_volatile(ch, 0);
        }
    }
    result
}

fn fill_credential_buffer<const N:usize>(buffer:&mut [u16;N],value:&str)->Result<(),String> {
    // Never silently truncate an existing password when presenting it for editing.
    if value.encode_utf16().count()>=N || value.contains('\0') {
        return Err("账号或密码超出 Windows 账号窗口的输入范围，原配置已保留。".into());
    }
    buffer.fill(0);
    for (slot,code) in buffer.iter_mut().zip(value.encode_utf16()) {*slot=code;}
    Ok(())
}

#[cfg(test)]
mod credential_tests {
    use super::*;
    use std::cell::RefCell;
    use windows_sys::Win32::{Foundation::*,System::Threading::GetCurrentThreadId,UI::{Controls::*,WindowsAndMessaging::*}};
    #[test]
    fn credential_prefill_does_not_truncate_unicode_or_accept_embedded_nul() {
        let mut data=[0u16;6];fill_credential_buffer(&mut data,"密😀码").unwrap();
        assert!(String::from_utf16(&data[..4]).unwrap()=="密😀码");assert_eq!(data[4],0);
        assert!(fill_credential_buffer(&mut data,"123456").is_err());
        assert!(fill_credential_buffer(&mut data,"a\0b").is_err());
        fill_credential_buffer(&mut data,"").unwrap();assert!(data.iter().all(|v|*v==0));
    }
    struct Scenario {secret:&'static str,remember:bool,cancel:bool,replacement:Option<&'static str>,checked:bool,attempts:u32}
    thread_local! {static SCENARIO:RefCell<Option<Scenario>>=const{RefCell::new(None)};}
    struct Controls {secret:HWND,remember:HWND}
    unsafe extern "system" fn child(window:HWND,param:LPARAM)->i32 {
        unsafe {
            let fields=&mut *(param as *mut Controls);let mut name=[0u16;64];let n=GetClassNameW(window,name.as_mut_ptr(),64);
            let class=String::from_utf16_lossy(&name[..n.max(0) as usize]);let style=GetWindowLongPtrW(window,GWL_STYLE) as u32;
            if class.eq_ignore_ascii_case("Edit") && style&ES_PASSWORD as u32!=0 {fields.secret=window;}
            if class.eq_ignore_ascii_case("Button") && [BS_CHECKBOX,BS_AUTOCHECKBOX].contains(&((style&BS_TYPEMASK as u32) as i32)) {fields.remember=window;}
        }1
    }
    unsafe extern "system" fn dialog(window:HWND,_:LPARAM)->i32 {
        unsafe {
            let mut fields=Controls {secret:std::ptr::null_mut(),remember:std::ptr::null_mut()};EnumChildWindows(window,Some(child),&mut fields as *mut _ as isize);
            if fields.secret.is_null() {return 1;}
            SCENARIO.with(|slot| {
                let mut slot=slot.borrow_mut();let state=slot.as_mut().unwrap();
                let n=GetWindowTextLengthW(fields.secret);let mut value=vec![0u16;n.max(0) as usize+1];let n=GetWindowTextW(fields.secret,value.as_mut_ptr(),value.len() as i32);
                let matches=String::from_utf16_lossy(&value[..n.max(0) as usize])==state.secret;
                let masked=SendMessageW(fields.secret,EM_GETPASSWORDCHAR,0,0)!=0;
                let remember=!fields.remember.is_null() && (SendMessageW(fields.remember,BM_GETCHECK,0,0)==BST_CHECKED as isize)==state.remember;
                state.checked=matches&&masked&&remember;
                if state.checked {
                    if let Some(replacement)=state.replacement {SetWindowTextW(fields.secret,crate::wide(replacement).as_ptr());}
                    crate::network_tools_dialog::tests::capture(window,"credentials-prefilled");
                    PostMessageW(window,WM_COMMAND,if state.cancel{IDCANCEL}else{IDOK} as usize,0);
                } else {PostMessageW(window,WM_CLOSE,0,0);}
            });
        }0
    }
    unsafe extern "system" fn inspect(_:HWND,_:u32,id:usize,_:u32) {
        unsafe {
            SCENARIO.with(|slot| {if let Some(state)=slot.borrow_mut().as_mut(){state.attempts+=1;}});
            EnumThreadWindows(GetCurrentThreadId(),Some(dialog),0);
            if SCENARIO.with(|slot|slot.borrow().as_ref().is_some_and(|s|s.checked||s.attempts>30)) {KillTimer(std::ptr::null_mut(),id);}
        }
    }
    #[test]
    #[ignore="Shows only synthetic credentials on an invisible test desktop; verifies masking, remember state, unchanged confirmation and cancellation; does not read or save real credentials"]
    fn native_credentials_prefill_and_cancel() {
        let _desktop=crate::menu_summary::tests::TestDesktop::new();
        for (remember,cancel,replacement) in [(true,false,None),(false,false,Some("replacement-test-secret")),(true,true,Some("discarded-test-secret"))] {
            SCENARIO.with(|slot| *slot.borrow_mut()=Some(Scenario {secret:"synthetic-test-secret",remember,cancel,replacement,checked:false,attempts:0}));
            let timer=unsafe {SetTimer(std::ptr::null_mut(),0,150,Some(inspect))};assert_ne!(timer,0);
            let result=prompt_credentials(std::ptr::null_mut(),"synthetic-account","synthetic-test-secret",remember);
            unsafe {KillTimer(std::ptr::null_mut(),timer);}
            let state=SCENARIO.with(|slot|slot.borrow_mut().take().unwrap());assert!(state.checked,"native password must be prefilled, masked and have correct remember state");
            let value=result.unwrap();
            if cancel {assert!(value.is_none());} else {let (user,password,saved)=value.unwrap();assert!(user=="synthetic-account"&&password==replacement.unwrap_or("synthetic-test-secret")&&saved==remember);}
        }
    }
}

pub fn protect_user_data(data: &[u8], protect: bool) -> Result<Vec<u8>, String> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
    let success = unsafe {
        if protect {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if success == 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        // DPAPI owns this allocation; clear the native plaintext before release.
        if !protect {
            for offset in 0..output.cbData as usize {
                std::ptr::write_volatile(output.pbData.add(offset), 0);
            }
            std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
        }
        LocalFree(output.pbData.cast());
    }
    Ok(result)
}

pub fn read_proxy() -> Result<(bool, String), String> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(INTERNET_SETTINGS, KEY_READ)
        .map_err(|err| err.to_string())?;
    let enabled = key.get_value::<u32, _>("ProxyEnable").unwrap_or(0) != 0;
    let server = key
        .get_value::<String, _>("ProxyServer")
        .unwrap_or_default();
    Ok((enabled, server))
}

pub fn set_proxy(enabled: bool, server: &str) -> Result<String, String> {
    if enabled && (server.trim().is_empty() || server.chars().any(char::is_whitespace)) {
        return Err("请填写代理地址，例如 127.0.0.1:7890。".to_owned());
    }
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(INTERNET_SETTINGS)
        .map_err(|err| err.to_string())?;
    if enabled {
        key.set_value("ProxyServer", &server.trim().to_owned())
            .map_err(|err| err.to_string())?;
    }
    key.set_value("ProxyEnable", &(if enabled { 1u32 } else { 0u32 }))
        .map_err(|err| err.to_string())?;
    unsafe {
        InternetSetOptionW(
            std::ptr::null(),
            INTERNET_OPTION_SETTINGS_CHANGED,
            std::ptr::null_mut(),
            0,
        );
        InternetSetOptionW(
            std::ptr::null(),
            INTERNET_OPTION_REFRESH,
            std::ptr::null_mut(),
            0,
        );
    }
    Ok(if enabled {
        format!("系统代理已开启：{}。", server.trim())
    } else {
        "系统代理已关闭。".to_owned()
    })
}
