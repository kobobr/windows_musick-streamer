use std::thread;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::{
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::{
            RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT, VK_F1, VK_F2, VK_F3,
        },
        WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY},
    },
};

use crate::{commands, state::AppState};

const HOTKEY_MUTE: i32 = 0x4E_01;
const HOTKEY_VOLUME_DOWN: i32 = 0x4E_02;
const HOTKEY_VOLUME_UP: i32 = 0x4E_03;
const HOTKEY_STEP: i16 = 5;

#[derive(Debug, Clone, Serialize)]
struct HotkeyVolumePayload {
    volume: u8,
    muted: bool,
}

pub fn start(app: AppHandle) {
    thread::spawn(move || {
        let registered_mute =
            unsafe { RegisterHotKey(None, HOTKEY_MUTE, MOD_NOREPEAT, VK_F1.0 as u32) }.is_ok();
        let registered_down =
            unsafe { RegisterHotKey(None, HOTKEY_VOLUME_DOWN, MOD_NOREPEAT, VK_F2.0 as u32) }
                .is_ok();
        let registered_up =
            unsafe { RegisterHotKey(None, HOTKEY_VOLUME_UP, MOD_NOREPEAT, VK_F3.0 as u32) }.is_ok();

        if !registered_mute && !registered_down && !registered_up {
            return;
        }

        let _thread_id = unsafe { GetCurrentThreadId() };
        let mut msg = MSG::default();
        loop {
            let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
            if result.0 <= 0 {
                break;
            }
            if msg.message != WM_HOTKEY {
                continue;
            }

            let enabled = app
                .state::<AppState>()
                .hotkeys_enabled
                .lock()
                .map(|enabled| *enabled)
                .unwrap_or(false);
            if !enabled {
                continue;
            }

            match msg.wParam.0 as i32 {
                HOTKEY_MUTE => {
                    if let Ok((volume, muted)) =
                        commands::toggle_mute_for_state(&app.state::<AppState>())
                    {
                        let _ = app.emit(
                            "hotkey-volume-changed",
                            HotkeyVolumePayload { volume, muted },
                        );
                        commands::emit_app_state_changed(&app);
                    }
                }
                HOTKEY_VOLUME_DOWN | HOTKEY_VOLUME_UP => {
                    let delta = if msg.wParam.0 as i32 == HOTKEY_VOLUME_DOWN {
                        -HOTKEY_STEP
                    } else {
                        HOTKEY_STEP
                    };
                    if let Ok(volume) =
                        commands::adjust_all_network_stream_volumes(&app.state::<AppState>(), delta)
                    {
                        let muted = app
                            .state::<AppState>()
                            .muted
                            .lock()
                            .map(|muted| *muted)
                            .unwrap_or(false);
                        let _ = app.emit(
                            "hotkey-volume-changed",
                            HotkeyVolumePayload { volume, muted },
                        );
                        commands::emit_app_state_changed(&app);
                    }
                }
                _ => continue,
            }
        }

        if registered_mute {
            let _ = unsafe { UnregisterHotKey(None, HOTKEY_MUTE) };
        }
        if registered_down {
            let _ = unsafe { UnregisterHotKey(None, HOTKEY_VOLUME_DOWN) };
        }
        if registered_up {
            let _ = unsafe { UnregisterHotKey(None, HOTKEY_VOLUME_UP) };
        }
    });
}
