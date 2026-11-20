use std::io;
use std::path::PathBuf;
use std::ptr::{null_mut, NonNull};
use winapi::ctypes::c_void;
use winapi::shared::ntdef::HRESULT;
use winapi::um::combaseapi::{CoCreateInstance, CoInitializeEx, CoUninitialize};
use winapi::um::objbase::COINIT_APARTMENTTHREADED;
use winapi::um::objidl::IPersistFile;
use winapi::um::shlobj::{SHGetFolderPathW, CSIDL_DESKTOPDIRECTORY};
use winapi::um::shobjidl_core::{IShellLinkW, ShellLink};
use winapi::{Class, Interface};

const RPC_E_CHANGED_MODE: HRESULT = 0x80010106u32 as i32;

pub fn create_desktop_shortcut() -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let shortcut = desktop_shortcut_path()?;
    if shortcut.exists() {
        return Ok(());
    }
    write_desktop_shortcut(&executable, &shortcut)
}

pub fn set_desktop_shortcut_target(executable: &std::path::Path) -> io::Result<()> {
    let shortcut = desktop_shortcut_path()?;
    write_desktop_shortcut(executable, &shortcut)
}

fn desktop_shortcut_path() -> io::Result<PathBuf> {
    let mut desktop = [0u16; 260];
    let folder_result = unsafe {
        SHGetFolderPathW(
            null_mut(),
            CSIDL_DESKTOPDIRECTORY,
            null_mut(),
            0,
            desktop.as_mut_ptr(),
        )
    };
    check_hresult(folder_result, "无法定位桌面目录")?;
    let folder_len = desktop
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(desktop.len());
    let mut shortcut = PathBuf::from(String::from_utf16_lossy(&desktop[..folder_len]));
    shortcut.push("绿玩变异工具箱.lnk");
    Ok(shortcut)
}

fn write_desktop_shortcut(
    executable: &std::path::Path,
    shortcut: &std::path::Path,
) -> io::Result<()> {
    let initialized = unsafe { CoInitializeEx(null_mut(), COINIT_APARTMENTTHREADED as u32) };
    if initialized < 0 && initialized != RPC_E_CHANGED_MODE {
        return Err(io::Error::other(format!(
            "初始化快捷方式组件失败：HRESULT 0x{:08X}",
            initialized as u32
        )));
    }
    let uninitialize = initialized >= 0;
    let result = unsafe { write_shortcut(executable, shortcut) };
    if uninitialize {
        unsafe { CoUninitialize() };
    }
    result
}

unsafe fn write_shortcut(
    executable: &std::path::Path,
    shortcut: &std::path::Path,
) -> io::Result<()> {
    let class_id = ShellLink::uuidof();
    let interface_id = IShellLinkW::uuidof();
    let mut raw_link: *mut c_void = null_mut();
    check_hresult(
        CoCreateInstance(
            &class_id,
            null_mut(),
            1,
            &interface_id,
            &mut raw_link as *mut *mut c_void,
        ),
        "创建快捷方式对象失败",
    )?;
    let link = NonNull::new(raw_link.cast::<IShellLinkW>())
        .ok_or_else(|| io::Error::other("创建快捷方式对象返回空指针"))?;
    let executable_wide = wide_path(executable);
    let description = wide("绿玩变异工具箱");
    let path_result = link.as_ref().SetPath(executable_wide.as_ptr());
    if path_result >= 0 {
        let _ = link.as_ref().SetDescription(description.as_ptr());
        let _ = link.as_ref().SetIconLocation(executable_wide.as_ptr(), 0);
    }
    if path_result < 0 {
        link.as_ref().Release();
        return Err(hresult_error(path_result, "设置快捷方式目标失败"));
    }

    let mut raw_persist: *mut c_void = null_mut();
    let persist_result = link.as_ref().QueryInterface(
        &IPersistFile::uuidof(),
        &mut raw_persist as *mut *mut c_void,
    );
    if persist_result < 0 {
        link.as_ref().Release();
        return Err(hresult_error(persist_result, "保存快捷方式接口失败"));
    }
    let persist = NonNull::new(raw_persist.cast::<IPersistFile>())
        .ok_or_else(|| io::Error::other("保存快捷方式接口返回空指针"))?;
    let shortcut_wide = wide_path(shortcut);
    let save_result = persist.as_ref().Save(shortcut_wide.as_ptr(), 1);
    persist.as_ref().Release();
    link.as_ref().Release();
    check_hresult(save_result, "写入桌面快捷方式失败")
}

fn check_hresult(result: HRESULT, context: &str) -> io::Result<()> {
    if result < 0 {
        Err(hresult_error(result, context))
    } else {
        Ok(())
    }
}

fn hresult_error(result: HRESULT, context: &str) -> io::Error {
    io::Error::other(format!("{context}：HRESULT 0x{:08X}", result as u32))
}

fn wide_path(path: &std::path::Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

use std::os::windows::ffi::OsStrExt;
