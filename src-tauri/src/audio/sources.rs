use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use windows::{
    core::BOOL,
    Win32::{
        Foundation::{HWND, LPARAM},
        UI::WindowsAndMessaging::{
            EnumWindows, GetAncestor, GetClassNameW, GetForegroundWindow, GetLastActivePopup,
            GetWindow, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW,
            GetWindowThreadProcessId, IsWindowVisible, GA_ROOTOWNER, GWL_EXSTYLE, GW_OWNER,
            WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
        },
    },
};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ApplicationCaptureTarget {
    pub id: String,
    pub name: String,
    pub process_id: u32,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureSource {
    #[default]
    System,
    Applications {
        apps: Vec<ApplicationCaptureTarget>,
    },
}

impl CaptureSource {
    pub fn label(&self) -> String {
        match self {
            Self::System => "System audio".to_string(),
            Self::Applications { apps } if apps.len() == 1 => apps[0].name.clone(),
            Self::Applications { apps } => format!("{} apps", apps.len()),
        }
    }

    fn selected_process_ids(&self) -> Vec<u32> {
        match self {
            Self::System => Vec::new(),
            Self::Applications { apps } => apps.iter().map(|app| app.process_id).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioSourceInfo {
    pub id: String,
    pub name: String,
    pub process_id: Option<u32>,
    pub kind: String,
    pub selected: bool,
}

pub fn system_source_info(selected: bool) -> AudioSourceInfo {
    AudioSourceInfo {
        id: "system".to_string(),
        name: "System audio".to_string(),
        process_id: None,
        kind: "system".to_string(),
        selected,
    }
}

pub fn list_audio_sources(selected: &CaptureSource) -> Vec<AudioSourceInfo> {
    let selected_pids = selected.selected_process_ids();
    let mut sources = vec![system_source_info(matches!(
        selected,
        CaptureSource::System
    ))];
    sources.extend(
        list_window_sources()
            .into_iter()
            .map(|target| AudioSourceInfo {
                selected: selected_pids.contains(&target.process_id),
                id: target.id,
                name: target.name,
                process_id: Some(target.process_id),
                kind: "application".to_string(),
            }),
    );
    sources
}

pub fn source_from_app_ids(ids: &[String], known_sources: &[AudioSourceInfo]) -> CaptureSource {
    if ids.is_empty() || ids.iter().any(|id| id == "system") {
        return CaptureSource::System;
    }

    let known = known_sources
        .iter()
        .filter_map(|source| {
            source.process_id.map(|process_id| {
                (
                    source.id.clone(),
                    ApplicationCaptureTarget {
                        id: source.id.clone(),
                        name: source.name.clone(),
                        process_id,
                    },
                )
            })
        })
        .collect::<HashMap<_, _>>();

    let mut apps = Vec::<ApplicationCaptureTarget>::new();
    for id in ids {
        let Some(app) = known.get(id) else {
            continue;
        };
        if apps
            .iter()
            .any(|candidate| candidate.process_id == app.process_id)
        {
            continue;
        }
        apps.push(app.clone());
    }

    if apps.is_empty() {
        CaptureSource::System
    } else {
        CaptureSource::Applications { apps }
    }
}

pub fn compact_source_label(label: &str) -> String {
    const MAX_LEN: usize = 54;
    let mut label = label.trim().replace('\n', " ");
    if label.chars().count() > MAX_LEN {
        label = label.chars().take(MAX_LEN - 1).collect::<String>();
        label.push_str("...");
    }
    label
}

pub fn source_menu_id(source: &AudioSourceInfo) -> String {
    if source.kind == "system" {
        "source:system".to_string()
    } else {
        source.id.clone()
    }
}

pub fn toggle_source_from_menu_id(
    menu_id: &str,
    current: &CaptureSource,
    known_sources: &[AudioSourceInfo],
) -> Option<CaptureSource> {
    if menu_id == "source:system" {
        return Some(CaptureSource::System);
    }

    let clicked = known_sources
        .iter()
        .find(|source| source.id == menu_id && source.kind == "application")?;
    let process_id = clicked.process_id?;

    let mut apps = match current {
        CaptureSource::Applications { apps } => apps.clone(),
        CaptureSource::System => Vec::new(),
    };

    if apps.iter().any(|app| app.process_id == process_id) {
        apps.retain(|app| app.process_id != process_id);
    } else {
        apps.push(ApplicationCaptureTarget {
            id: clicked.id.clone(),
            name: clicked.name.clone(),
            process_id,
        });
    }

    Some(if apps.is_empty() {
        CaptureSource::System
    } else {
        CaptureSource::Applications { apps }
    })
}

fn is_main_window(hwnd: HWND) -> bool {
    if !unsafe { IsWindowVisible(hwnd).as_bool() } {
        return false;
    }

    let exstyle = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 };
    if exstyle & WS_EX_TOOLWINDOW.0 != 0 {
        return false;
    }

    let owner = unsafe { GetWindow(hwnd, GW_OWNER) };
    if owner.is_ok() && exstyle & WS_EX_APPWINDOW.0 == 0 {
        let root_owner = unsafe { GetAncestor(hwnd, GA_ROOTOWNER) };
        let last_popup = unsafe { GetLastActivePopup(root_owner) };
        if last_popup != hwnd {
            return false;
        }
    }

    let title_len = unsafe { GetWindowTextLengthW(hwnd) };
    if title_len <= 0 {
        return false;
    }

    let mut buffer = vec![0_u16; title_len as usize + 1];
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    if copied <= 0 {
        return false;
    }
    let title = String::from_utf16_lossy(&buffer[..copied as usize])
        .trim()
        .to_string();
    if title.is_empty() || title == "Program Manager" {
        return false;
    }

    let foreground = unsafe { GetForegroundWindow() };
    if foreground == hwnd {
        return true;
    }

    let mut class_name = [0u16; 256];
    let class_len = unsafe { GetClassNameW(hwnd, &mut class_name) };
    if class_len > 0 {
        let class = String::from_utf16_lossy(&class_name[..class_len as usize]);
        if class.contains("Shell_TrayWnd")
            || class.contains("Shell_SecondaryTrayWnd")
            || class.contains("Progman")
            || class.contains("Windows.UI.Core.CoreWindow")
        {
            return false;
        }
    }

    true
}

fn list_window_sources() -> Vec<ApplicationCaptureTarget> {
    let mut windows = Vec::<ApplicationCaptureTarget>::new();
    let lparam = LPARAM(&mut windows as *mut _ as isize);
    let _ = unsafe { EnumWindows(Some(enum_windows_proc), lparam) };

    let mut by_pid = HashMap::<u32, ApplicationCaptureTarget>::new();
    for window in windows {
        by_pid.entry(window.process_id).or_insert(window);
    }

    let mut sources = by_pid.into_values().collect::<Vec<_>>();
    sources.sort_by_key(|source| source.name.to_lowercase());
    sources
}

unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if !is_main_window(hwnd) {
        return true.into();
    }

    let title_len = unsafe { GetWindowTextLengthW(hwnd) };
    if title_len <= 0 {
        return true.into();
    }

    let mut buffer = vec![0_u16; title_len as usize + 1];
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    if copied <= 0 {
        return true.into();
    }
    let title = String::from_utf16_lossy(&buffer[..copied as usize])
        .trim()
        .to_string();
    if title.is_empty() || title == "Program Manager" {
        return true.into();
    }

    let mut process_id = 0_u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    if process_id == 0 {
        return true.into();
    }

    let sources = unsafe { &mut *(lparam.0 as *mut Vec<ApplicationCaptureTarget>) };
    sources.push(ApplicationCaptureTarget {
        id: format!("source:app:{process_id}"),
        name: compact_source_label(&title),
        process_id,
    });

    true.into()
}
