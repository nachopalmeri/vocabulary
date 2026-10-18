#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(not(windows), allow(dead_code, unused_imports))]
#![allow(dead_code, unused_imports)]
#![allow(
    clippy::collapsible_if,
    clippy::derivable_impls,
    clippy::field_reassign_with_default,
    clippy::question_mark,
    clippy::unnecessary_cast,
    clippy::redundant_closure,
    clippy::useless_vec,
    clippy::borrowed_box,
    clippy::needless_borrow
)]

mod app;
mod csvio;
mod model;
mod rotation;
mod samples;
mod startup;
mod store;
mod theme;
mod tray;
mod ui;
mod win;

#[cfg(windows)]
fn main() {
    if let Err(error) = run() {
        crate::win::error(
            windows::Win32::Foundation::HWND::default(),
            &error,
            "WordDock",
        );
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("WordDock is a Windows-only application.");
}

#[cfg(windows)]
fn run() -> Result<(), String> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, LPARAM, WPARAM};
    use windows::Win32::System::RemoteDesktop::{
        NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
    };
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DispatchMessageW, FindWindowW, GWLP_USERDATA, GetMessageW, MSG,
        PostMessageW, SetWindowLongPtrW, TranslateMessage, WM_QUIT, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_POPUP,
    };

    const INSTANCE_MUTEX: &str = "Local\\WordDock.SingleInstance";
    let mutex_name = win::wide(INSTANCE_MUTEX);
    // SAFETY: the name is a valid NUL-terminated string. Keeping this handle open
    // owns the named mutex for the process lifetime.
    let mutex = unsafe { CreateMutexW(None, true, win::pcw(&mutex_name)) }
        .map_err(|error| format!("Could not create the single-instance mutex: {error}"))?;
    if unsafe { windows::Win32::Foundation::GetLastError() } == ERROR_ALREADY_EXISTS {
        // Reuse the already registered controller class/window and reveal it.
        let class = win::wide(app::CONTROLLER_CLASS);
        let existing = unsafe { FindWindowW(win::pcw(&class), windows::core::PCWSTR::null()) }
            .unwrap_or_default();
        if !existing.0.is_null() {
            unsafe {
                let _ = PostMessageW(Some(existing), app::WM_APP_REVEAL, WPARAM(0), LPARAM(0));
            }
        }
        unsafe {
            let _ = CloseHandle(mutex);
        }
        return Ok(());
    }

    // Per-monitor-v2 is also embedded in the manifest; this call is a fallback
    // for launchers that may alter the process default.
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let instance = win::module_instance();
    register_controller_class(instance)?;
    for spec in [
        ui::card::class(),
        ui::settings::class(),
        ui::collections::class(),
        ui::collections::prompt_class(),
        ui::details::class(),
        ui::import::class(),
    ] {
        if !ui::register_class(&spec, instance) {
            unsafe {
                let _ = CloseHandle(mutex);
            }
            return Err(format!(
                "Could not register the {} window class.",
                spec.name
            ));
        }
    }

    // Create an off-screen, non-activating controller. It has no visible
    // surface; using it as tray owner prevents the card stealing focus.
    let class = win::wide(app::CONTROLLER_CLASS);
    let controller = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            win::pcw(&class),
            windows::core::PCWSTR::null(),
            WS_POPUP,
            -32000,
            -32000,
            1,
            1,
            None,
            None,
            Some(instance),
            None,
        )
    }
    .map_err(|error| format!("Could not create the WordDock controller window: {error}"))?;

    let paths = store::Paths::discover();
    let mut app = app::App::new(controller, paths);
    unsafe {
        SetWindowLongPtrW(
            controller,
            GWLP_USERDATA,
            &mut app as *mut app::App as isize,
        );
    }
    if let Some(fatal) = app.fatal_error.clone() {
        win::error(controller, &fatal, "WordDock cannot start");
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(controller);
        }
        drop(app);
        unsafe {
            let _ = CloseHandle(mutex);
        }
        return Ok(());
    }
    let _ = ui::card::create(&mut app, instance);
    let _ = app
        .tray
        .add(controller, app::WM_APP_TRAY, "WordDock · vocabulary card");

    if let Err(error) =
        unsafe { WTSRegisterSessionNotification(controller, NOTIFY_FOR_THIS_SESSION) }
    {
        app.queue_notice(format!("Session-lock notifications could not be registered. Rotation may continue while Windows is locked.\n\n{error}"), true);
    }

    if app.config.card_visible {
        app.show_card();
    }
    app.restart_timer();

    let mut message = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        if result <= 0 {
            break;
        }
        if !ui::is_dialog_message(&mut message) {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if message.message == WM_QUIT {
            break;
        }
    }

    app.tray.remove(controller);
    let _ = unsafe {
        windows::Win32::System::RemoteDesktop::WTSUnRegisterSessionNotification(controller)
    };
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(controller);
    }
    // Keep `app` alive through window teardown, then release the mutex.
    drop(app);
    unsafe {
        let _ = CloseHandle(mutex);
    }
    Ok(())
}

#[cfg(windows)]
fn register_controller_class(
    instance: windows::Win32::Foundation::HINSTANCE,
) -> Result<(), String> {
    use windows::Win32::Graphics::Gdi::HBRUSH;
    use windows::Win32::UI::WindowsAndMessaging::{RegisterClassExW, WNDCLASS_STYLES, WNDCLASSEXW};
    let name = win::wide(app::CONTROLLER_CLASS);
    let class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: WNDCLASS_STYLES(0),
        lpfnWndProc: Some(controller_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: Default::default(),
        hCursor: win::load_cursor(windows::Win32::UI::WindowsAndMessaging::IDC_ARROW),
        hbrBackground: HBRUSH::default(),
        lpszMenuName: windows::core::PCWSTR::null(),
        lpszClassName: win::pcw(&name),
        hIconSm: Default::default(),
    };
    // SAFETY: class name is NUL-terminated and the descriptor lives through call.
    if unsafe { RegisterClassExW(&class) } == 0 {
        let error = unsafe { windows::Win32::Foundation::GetLastError() };
        if error != windows::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS {
            return Err(format!(
                "Could not register the controller window class: {error:?}"
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
unsafe extern "system" fn controller_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    // Existing controller_proc is safe extern; trampoline preserves the Win32 ABI.
    app::controller_proc(hwnd, msg, wparam, lparam)
}
