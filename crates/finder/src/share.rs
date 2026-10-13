//! The toolbar's share button: Windows' share sheet for the selection,
//! reached the way Explorer's own Share command is, through the items'
//! shell context menu and its "Windows.ModernShare" verb.

use std::path::PathBuf;

use windows::{
    Win32::{
        Foundation::HWND,
        UI::{
            Shell::{
                BHID_SFUIObject, CMF_NORMAL, CMINVOKECOMMANDINFO, Common::ITEMIDLIST, IContextMenu, ILFree, IShellItemArray,
                SHCreateShellItemArrayFromIDLists, SHParseDisplayName,
            },
            WindowsAndMessaging::{CreatePopupMenu, DestroyMenu, SW_SHOWNORMAL},
        },
    },
    core::{HSTRING, PCSTR},
};

/// Show the share sheet over window `hwnd` for `paths`.
pub fn share(hwnd: isize, paths: Vec<PathBuf>) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    unsafe {
        let mut pidls: Vec<*mut ITEMIDLIST> = Vec::with_capacity(paths.len());
        for p in &paths {
            let mut pidl = std::ptr::null_mut();
            if SHParseDisplayName(&HSTRING::from(p.as_os_str()), None, &mut pidl, 0, None).is_ok() {
                pidls.push(pidl);
            }
        }
        let result = invoke(HWND(hwnd as *mut _), &pidls);
        for pidl in pidls {
            ILFree(Some(pidl));
        }
        result.map_err(|e| e.message())
    }
}

unsafe fn invoke(hwnd: HWND, pidls: &[*mut ITEMIDLIST]) -> windows::core::Result<()> {
    let ids: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const _).collect();
    unsafe {
        let items: IShellItemArray = SHCreateShellItemArrayFromIDLists(&ids)?;
        let menu: IContextMenu = items.BindToHandler(None, &BHID_SFUIObject)?;
        // A context menu only knows its verbs once it has built itself.
        let popup = CreatePopupMenu()?;
        let built = menu.QueryContextMenu(popup, 0, 1, 0x7FFF, CMF_NORMAL);
        let verb = c"Windows.ModernShare";
        let info = CMINVOKECOMMANDINFO {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,
            hwnd,
            lpVerb: PCSTR(verb.as_ptr() as *const u8),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        let res = built.ok().and_then(|()| menu.InvokeCommand(&info));
        let _ = DestroyMenu(popup);
        res
    }
}
